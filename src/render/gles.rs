use glow::HasContext;
use smithay::backend::renderer::element::{Element, Id, RenderElement};
use smithay::backend::renderer::gles::{GlesError, GlesTexture};
use smithay::backend::renderer::glow::{GlowFrame, GlowRenderer};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size};
use tracing::debug;

use crate::{orbital::OrbitalSwitcher, render::space::Starfield};

// Shader for drawing a window thumbnail as a textured circle.
// The quad covers the planet bounding box; the fragment clips to a disc
// using the uv distance and also adds an outer glow ring.
const THUMB_VERT: &str = r#"
    attribute vec2 a_pos;
    attribute vec2 a_uv;
    varying vec2 v_uv;
    uniform vec2 u_screen_size;
    void main() {
        vec2 ndc = (a_pos / u_screen_size) * 2.0 - 1.0;
        ndc.y = -ndc.y;
        gl_Position = vec4(ndc, 0.0, 1.0);
        v_uv = a_uv;
    }
"#;

const THUMB_FRAG: &str = r#"
    precision mediump float;
    varying vec2 v_uv;
    uniform sampler2D u_texture;
    uniform float u_alpha;
    uniform vec4 u_border_color;
    void main() {
        // v_uv goes 0..1 across the quad; map to -1..1 for distance test
        vec2 centered = v_uv * 2.0 - 1.0;
        float dist = length(centered);

        // Clip outside the circle (with soft anti-aliased edge)
        float circle_alpha = 1.0 - smoothstep(0.92, 1.0, dist);

        // Border ring near the edge
        float border = smoothstep(0.85, 0.90, dist) * (1.0 - smoothstep(0.92, 1.0, dist));

        vec4 tex_color = texture2D(u_texture, v_uv);
        // Blend: texture inside, border colour near edge
        vec4 color = mix(tex_color, u_border_color, border * u_border_color.a);
        gl_FragColor = vec4(color.rgb, color.a * circle_alpha * u_alpha);
    }
"#;

const STAR_VERT: &str = r#"
    attribute vec2  a_pos;
    attribute float a_brightness;
    attribute float a_phase;
    varying   float v_brightness;
    uniform vec2  u_camera_offset;
    uniform float u_parallax_factor;
    uniform float u_size_scale;
    uniform float u_time;
    void main() {
        // Each layer scrolls the camera offset by its parallax factor.
        vec2 uv  = fract(a_pos - u_camera_offset * u_parallax_factor);
        vec2 ndc = uv * 2.0 - 1.0;
        ndc.y    = -ndc.y;
        gl_Position = vec4(ndc, 0.0, 1.0);
        // Twinkle: sine-wave flicker per star using its phase offset.
        float twinkle   = sin(u_time * 1.5 + a_phase) * 0.12;
        float effective = clamp(a_brightness + twinkle, 0.0, 1.0);
        v_brightness    = effective;
        gl_PointSize    = mix(1.0, 3.5, effective) * u_size_scale;
    }
"#;

const STAR_FRAG: &str = r#"
    precision mediump float;
    varying float v_brightness;
    void main() {
        float d     = length(gl_PointCoord - vec2(0.5));
        float alpha = (1.0 - smoothstep(0.3, 0.5, d)) * v_brightness;
        // Slight blue tint for far stars, warmer for near ones.
        gl_FragColor = vec4(0.82 + v_brightness * 0.10,
                            0.88 + v_brightness * 0.06,
                            1.00,
                            alpha);
    }
"#;

const GEOM_VERT: &str = r#"
    attribute vec2 a_pos;
    uniform   vec2 u_screen_size;
    void main() {
        vec2 ndc  = (a_pos / u_screen_size) * 2.0 - 1.0;
        ndc.y     = -ndc.y;
        gl_Position = vec4(ndc, 0.0, 1.0);
    }
"#;

const GEOM_FRAG: &str = r#"
    precision mediump float;
    uniform vec4 u_color;
    void main() { gl_FragColor = u_color; }
"#;

unsafe fn compile_program(gl: &glow::Context, vert: &str, frag: &str) -> anyhow::Result<glow::Program> {
    let vs = gl.create_shader(glow::VERTEX_SHADER).map_err(|e| anyhow::anyhow!("{e}"))?;
    gl.shader_source(vs, vert);
    gl.compile_shader(vs);
    if !gl.get_shader_compile_status(vs) { let l = gl.get_shader_info_log(vs); gl.delete_shader(vs); anyhow::bail!("vert: {l}"); }

    let fs = gl.create_shader(glow::FRAGMENT_SHADER).map_err(|e| anyhow::anyhow!("{e}"))?;
    gl.shader_source(fs, frag);
    gl.compile_shader(fs);
    if !gl.get_shader_compile_status(fs) { let l = gl.get_shader_info_log(fs); gl.delete_shader(vs); gl.delete_shader(fs); anyhow::bail!("frag: {l}"); }

    let prog = gl.create_program().map_err(|e| anyhow::anyhow!("{e}"))?;
    gl.attach_shader(prog, vs); gl.attach_shader(prog, fs);
    gl.link_program(prog);
    gl.delete_shader(vs); gl.delete_shader(fs);
    if !gl.get_program_link_status(prog) { let l = gl.get_program_info_log(prog); gl.delete_program(prog); anyhow::bail!("link: {l}"); }
    Ok(prog)
}

/// Per-layer GPU buffer for the parallax starfield.
#[derive(Copy, Clone)]
struct StarLayerBuf {
    vbo:   glow::Buffer,
    count: i32,
}

pub struct GlesSpaceRenderer {
    // Parallax starfield (3 layers, same shader program)
    star_prog:      glow::Program,
    star_layers:    [StarLayerBuf; 3],
    star_a_pos:     u32,
    star_a_bright:  u32,
    star_a_phase:   u32,
    star_u_camera:  glow::UniformLocation,
    star_u_parallax: glow::UniformLocation,
    star_u_size:    glow::UniformLocation,
    star_u_time:    glow::UniformLocation,
    // RenderElement identity (stable across frames so damage tracking works)
    starfield_id:   Id,
    star_commit:    CommitCounter,
    // Geometry (orbit rings, halos, etc.)
    geom_prog:     glow::Program,
    geom_a_pos:    u32,
    geom_u_screen: glow::UniformLocation,
    geom_u_color:  glow::UniformLocation,
    // Thumbnail (textured circle) shader
    thumb_prog:      glow::Program,
    thumb_a_pos:     u32,
    thumb_a_uv:      u32,
    thumb_u_screen:  glow::UniformLocation,
    thumb_u_texture: glow::UniformLocation,
    thumb_u_alpha:   glow::UniformLocation,
    thumb_u_border:  glow::UniformLocation,
    pub thumbnails: crate::render::thumbnail::ThumbnailCache,
}

impl GlesSpaceRenderer {
    pub fn init(renderer: &mut GlowRenderer, starfield: &Starfield) -> anyhow::Result<Self> {
        let mut out: Option<anyhow::Result<Self>> = None;
        renderer.with_context(|gl| { out = Some(unsafe { Self::init_gl(gl, starfield) }); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        out.unwrap()
    }

    unsafe fn init_gl(gl: &glow::Context, starfield: &Starfield) -> anyhow::Result<Self> {
        let star_prog      = compile_program(gl, STAR_VERT, STAR_FRAG)?;
        let star_a_pos     = gl.get_attrib_location(star_prog, "a_pos").ok_or_else(|| anyhow::anyhow!("a_pos"))? as u32;
        let star_a_bright  = gl.get_attrib_location(star_prog, "a_brightness").ok_or_else(|| anyhow::anyhow!("a_brightness"))? as u32;
        let star_a_phase   = gl.get_attrib_location(star_prog, "a_phase").ok_or_else(|| anyhow::anyhow!("a_phase"))? as u32;
        let star_u_camera  = gl.get_uniform_location(star_prog, "u_camera_offset").ok_or_else(|| anyhow::anyhow!("u_camera_offset"))?;
        let star_u_parallax = gl.get_uniform_location(star_prog, "u_parallax_factor").ok_or_else(|| anyhow::anyhow!("u_parallax_factor"))?;
        let star_u_size    = gl.get_uniform_location(star_prog, "u_size_scale").ok_or_else(|| anyhow::anyhow!("u_size_scale"))?;
        let star_u_time    = gl.get_uniform_location(star_prog, "u_time").ok_or_else(|| anyhow::anyhow!("u_time"))?;

        // Upload one VBO per parallax layer: [x, y, brightness, phase] per star.
        let star_layers = {
            let layers_data = starfield.layers();
            let mut result: Vec<StarLayerBuf> = Vec::with_capacity(3);
            for layer in layers_data.iter() {
                let mut data: Vec<f32> = Vec::with_capacity(layer.stars.len() * 4);
                for s in &layer.stars {
                    data.push(s.pos.x);
                    data.push(s.pos.y);
                    data.push(s.brightness);
                    data.push(s.phase);
                }
                let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&data), glow::STATIC_DRAW);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                result.push(StarLayerBuf { vbo, count: layer.stars.len() as i32 });
            }
            let [a, b, c]: [StarLayerBuf; 3] = result.try_into()
                .map_err(|_| anyhow::anyhow!("expected 3 star layers"))?;
            [a, b, c]
        };

        let geom_prog     = compile_program(gl, GEOM_VERT, GEOM_FRAG)?;
        let geom_a_pos    = gl.get_attrib_location(geom_prog, "a_pos").ok_or_else(|| anyhow::anyhow!("geom a_pos"))? as u32;
        let geom_u_screen = gl.get_uniform_location(geom_prog, "u_screen_size").ok_or_else(|| anyhow::anyhow!("geom u_screen_size"))?;
        let geom_u_color  = gl.get_uniform_location(geom_prog, "u_color").ok_or_else(|| anyhow::anyhow!("geom u_color"))?;

        let thumb_prog    = compile_program(gl, THUMB_VERT, THUMB_FRAG)?;
        let thumb_a_pos   = gl.get_attrib_location(thumb_prog, "a_pos").ok_or_else(|| anyhow::anyhow!("thumb a_pos"))? as u32;
        let thumb_a_uv    = gl.get_attrib_location(thumb_prog, "a_uv").ok_or_else(|| anyhow::anyhow!("thumb a_uv"))? as u32;
        let thumb_u_screen  = gl.get_uniform_location(thumb_prog, "u_screen_size").ok_or_else(|| anyhow::anyhow!("thumb u_screen_size"))?;
        let thumb_u_texture = gl.get_uniform_location(thumb_prog, "u_texture").ok_or_else(|| anyhow::anyhow!("thumb u_texture"))?;
        let thumb_u_alpha   = gl.get_uniform_location(thumb_prog, "u_alpha").ok_or_else(|| anyhow::anyhow!("thumb u_alpha"))?;
        let thumb_u_border  = gl.get_uniform_location(thumb_prog, "u_border_color").ok_or_else(|| anyhow::anyhow!("thumb u_border_color"))?;

        let total_stars: i32 = star_layers.iter().map(|l| l.count).sum();
        debug!("GlesSpaceRenderer ready — {} stars (3 parallax layers)", total_stars);
        Ok(Self {
            star_prog, star_layers,
            star_a_pos, star_a_bright, star_a_phase,
            star_u_camera, star_u_parallax, star_u_size, star_u_time,
            starfield_id: Id::new(),
            star_commit: CommitCounter::default(),
            geom_prog, geom_a_pos, geom_u_screen, geom_u_color,
            thumb_prog, thumb_a_pos, thumb_a_uv,
            thumb_u_screen, thumb_u_texture, thumb_u_alpha, thumb_u_border,
            thumbnails: crate::render::thumbnail::ThumbnailCache::new(),
        })
    }

    /// Build a `StarfieldElement` that can be passed to `render_output` as a bottom-layer element.
    ///
    /// The element captures the current GL handles and per-frame animation state so that
    /// `render_output_internal` can call `draw()` on it at the correct z-order position.
    pub fn make_starfield_element(
        &mut self,
        starfield: &Starfield,
        cam_x: f32,
        cam_y: f32,
        width: i32,
        height: i32,
    ) -> StarfieldElement {
        self.star_commit.increment();
        let layers = starfield.layers();
        StarfieldElement {
            id: self.starfield_id.clone(),
            commit: self.star_commit,
            star_prog: self.star_prog,
            star_layers: [
                (self.star_layers[0].vbo, self.star_layers[0].count),
                (self.star_layers[1].vbo, self.star_layers[1].count),
                (self.star_layers[2].vbo, self.star_layers[2].count),
            ],
            star_a_pos:     self.star_a_pos,
            star_a_bright:  self.star_a_bright,
            star_a_phase:   self.star_a_phase,
            star_u_camera:  self.star_u_camera,
            star_u_parallax: self.star_u_parallax,
            star_u_size:    self.star_u_size,
            star_u_time:    self.star_u_time,
            cam_x,
            cam_y,
            time: starfield.time,
            layer_parallax: [
                layers[0].parallax_factor,
                layers[1].parallax_factor,
                layers[2].parallax_factor,
            ],
            layer_size: [
                layers[0].size_scale,
                layers[1].size_scale,
                layers[2].size_scale,
            ],
            width,
            height,
        }
    }

    /// DRM-backend variant: raw GL handle, no GlowFrame available.
    pub unsafe fn draw_starfield_gl(&self, gl: &glow::Context, starfield: &Starfield, cx: f32, cy: f32) {
        gl.clear_color(0.0, 0.0, 0.03, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.use_program(Some(self.star_prog));
        gl.uniform_2_f32(Some(&self.star_u_camera), cx, cy);
        gl.uniform_1_f32(Some(&self.star_u_time), starfield.time);
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE);
        let s = (4 * std::mem::size_of::<f32>()) as i32;
        let f32_size = std::mem::size_of::<f32>() as i32;
        for (layer_buf, layer_meta) in self.star_layers.iter().zip(starfield.layers()) {
            gl.uniform_1_f32(Some(&self.star_u_parallax), layer_meta.parallax_factor);
            gl.uniform_1_f32(Some(&self.star_u_size), layer_meta.size_scale);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(layer_buf.vbo));
            gl.enable_vertex_attrib_array(self.star_a_pos);
            gl.vertex_attrib_pointer_f32(self.star_a_pos, 2, glow::FLOAT, false, s, 0);
            gl.enable_vertex_attrib_array(self.star_a_bright);
            gl.vertex_attrib_pointer_f32(self.star_a_bright, 1, glow::FLOAT, false, s, 2 * f32_size);
            gl.enable_vertex_attrib_array(self.star_a_phase);
            gl.vertex_attrib_pointer_f32(self.star_a_phase, 1, glow::FLOAT, false, s, 3 * f32_size);
            gl.draw_arrays(glow::POINTS, 0, layer_buf.count);
            gl.disable_vertex_attrib_array(self.star_a_pos);
            gl.disable_vertex_attrib_array(self.star_a_bright);
            gl.disable_vertex_attrib_array(self.star_a_phase);
        }
        gl.disable(glow::BLEND);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.use_program(None);
    }

    // -----------------------------------------------------------------------
    // System view — orbital overlay for the active workspace
    // -----------------------------------------------------------------------

    pub fn draw_orbital_overlay(&self, frame: &mut GlowFrame<'_, '_>, screen: Size<i32, Physical>,
                                 orbital: &OrbitalSwitcher) -> anyhow::Result<()> {
        frame.with_context(|gl| unsafe { self.gl_draw_orbital(&**gl, screen, orbital); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    /// DRM-backend variant.
    pub unsafe fn draw_orbital_overlay_gl(&self, gl: &glow::Context, screen: Size<i32, Physical>, orbital: &OrbitalSwitcher) {
        self.gl_draw_orbital(gl, screen, orbital);
    }

    unsafe fn gl_draw_orbital(&self, gl: &glow::Context, screen: Size<i32, Physical>, orbital: &OrbitalSwitcher) {
        use crate::render::palette;
        let cam = &orbital.camera;
        let ws = orbital.active_ws();

        // --- Vignette during camera transitions ---
        let anim_alpha = {
            let dp = (cam.position - cam.target_position).length();
            let dz = (cam.zoom - cam.target_zoom).abs();
            // Max out at 0.35 opacity during strong transitions, fades as camera settles.
            ((dp * 0.001 + dz * 2.0) * 0.35).clamp(0.0, 0.35_f32)
        };
        if anim_alpha > 0.01 {
            gl.use_program(Some(self.geom_prog));
            gl.uniform_2_f32(Some(&self.geom_u_screen), screen.w as f32, screen.h as f32);
            gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.uniform_4_f32(Some(&self.geom_u_color), 0.0, 0.0, 0.05, anim_alpha);
            // Full-screen quad: two triangles covering 0..screen
            let sw = screen.w as f32; let sh = screen.h as f32;
            let quad: [f32; 12] = [0.0, 0.0,  sw, 0.0,  0.0, sh,
                                   0.0, sh,    sw, 0.0,  sw,  sh];
            let vbo = gl.create_buffer().expect("vignette vbo");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&quad), glow::STREAM_DRAW);
            gl.enable_vertex_attrib_array(self.geom_a_pos);
            gl.vertex_attrib_pointer_f32(self.geom_a_pos, 2, glow::FLOAT, false,
                (2 * std::mem::size_of::<f32>()) as i32, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
            gl.disable_vertex_attrib_array(self.geom_a_pos);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.delete_buffer(vbo);
            gl.disable(glow::BLEND);
            gl.use_program(None);
        }

        // --- Geometry pass (rings + sun corona) ---
        gl.use_program(Some(self.geom_prog));
        gl.uniform_2_f32(Some(&self.geom_u_screen), screen.w as f32, screen.h as f32);
        gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

        // Orbit rings
        let p = palette::ORBIT_RING;
        gl.uniform_4_f32(Some(&self.geom_u_color), p[0], p[1], p[2], p[3]);
        let max_ring = ws.planets.iter().map(|p| p.orbit_index).max().map(|m| m + 1).unwrap_or(0);
        for ring in 0..max_ring {
            let r = (crate::orbital::body::ORBIT_BASE_RADIUS + ring as f32 * crate::orbital::body::ORBIT_STEP) * cam.zoom;
            let c = cam.world_to_screen().transform_point2(ws.world_pos);
            self.draw_circle_line(gl, c.x, c.y, r, 64);
        }
        // Sun corona — pulsing glow
        if ws.sun.is_some() {
            let c = cam.world_to_screen().transform_point2(ws.world_pos);
            // Slow pulse: period ~3 s, amplitude ±8 %
            let pulse = 1.0 + (orbital.time * std::f32::consts::TAU / 3.0).sin() * 0.08;
            let base  = 80.0 * cam.zoom * pulse;
            // Inner ring — solid warm glow
            let inner = palette::SUN_INNER;
            gl.uniform_4_f32(Some(&self.geom_u_color), inner[0], inner[1], inner[2], inner[3]);
            self.draw_circle_line(gl, c.x, c.y, base, 64);
            // Outer halo — fades in/out with a second pulse (phase-shifted)
            let pulse2 = 1.0 + (orbital.time * std::f32::consts::TAU / 3.0 + 1.0).sin() * 0.12;
            let outer_alpha = palette::SUN_OUTER[3] * (0.55 + pulse2 * 0.45);
            let outer = palette::SUN_OUTER;
            gl.uniform_4_f32(Some(&self.geom_u_color), outer[0], outer[1], outer[2], outer_alpha);
            self.draw_circle_line(gl, c.x, c.y, base * 1.4, 64);
            // Second outer ring for extra depth, slower pulse
            let pulse3 = 1.0 + (orbital.time * std::f32::consts::TAU / 5.0).sin() * 0.06;
            let outer3_alpha = 0.18 * pulse3;
            gl.uniform_4_f32(Some(&self.geom_u_color), outer[0], outer[1], outer[2], outer3_alpha);
            self.draw_circle_line(gl, c.x, c.y, base * 2.0, 64);
        }
        gl.disable(glow::BLEND);
        gl.use_program(None);

        // --- Thumbnail pass (textured circles for planets) ---
        gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        for (i, planet) in ws.planets.iter().enumerate() {
            let planet_world = ws.world_pos + planet.world_pos();
            let sp  = cam.world_to_screen().transform_point2(planet_world);
            let r   = planet.visual_diameter() * 0.5 * cam.zoom;
            let col = if orbital.hovered_planet == Some(i) { palette::PLANET_HOVER } else { palette::PLANET_BORDER };

            if let Some(tex) = self.thumbnails.get(&planet.window) {
                self.draw_thumbnail_circle(gl, screen, sp.x, sp.y, r, tex, col, planet.alpha);
            } else {
                // No thumbnail yet — fall back to plain ring
                gl.use_program(Some(self.geom_prog));
                gl.uniform_2_f32(Some(&self.geom_u_screen), screen.w as f32, screen.h as f32);
                gl.uniform_4_f32(Some(&self.geom_u_color), col[0], col[1], col[2], col[3] * planet.alpha);
                self.draw_circle_line(gl, sp.x, sp.y, r, 32);
                gl.use_program(None);
            }
        }
        gl.disable(glow::BLEND);
    }

    /// Draw a textured circle (planet thumbnail) at screen-space position (cx, cy) with radius r.
    unsafe fn draw_thumbnail_circle(
        &self,
        gl: &glow::Context,
        screen: Size<i32, Physical>,
        cx: f32, cy: f32, r: f32,
        tex: &GlesTexture,
        border_color: [f32; 4],
        alpha: f32,
    ) {
        gl.use_program(Some(self.thumb_prog));
        gl.uniform_2_f32(Some(&self.thumb_u_screen), screen.w as f32, screen.h as f32);
        gl.uniform_1_i32(Some(&self.thumb_u_texture), 0);
        gl.uniform_1_f32(Some(&self.thumb_u_alpha), alpha);
        gl.uniform_4_f32(Some(&self.thumb_u_border),
            border_color[0], border_color[1], border_color[2], border_color[3]);

        // Bind the thumbnail texture to unit 0.
        // GlesTexture::tex_id() → raw GLuint; glow::Texture is a NonZeroU32 newtype.
        let raw_tex = std::num::NonZeroU32::new(tex.tex_id())
            .map(glow::NativeTexture);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, raw_tex);

        // Build a quad covering the bounding box of the circle (cx±r, cy±r).
        // Each vertex: [screen_x, screen_y, u, v]
        let x0 = cx - r; let x1 = cx + r;
        let y0 = cy - r; let y1 = cy + r;
        let verts: [f32; 24] = [
            x0, y0,  0.0, 0.0,
            x1, y0,  1.0, 0.0,
            x0, y1,  0.0, 1.0,
            x0, y1,  0.0, 1.0,
            x1, y0,  1.0, 0.0,
            x1, y1,  1.0, 1.0,
        ];

        let vbo = gl.create_buffer().expect("thumb vbo");
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&verts), glow::STREAM_DRAW);

        let stride = (4 * std::mem::size_of::<f32>()) as i32;
        gl.enable_vertex_attrib_array(self.thumb_a_pos);
        gl.vertex_attrib_pointer_f32(self.thumb_a_pos, 2, glow::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(self.thumb_a_uv);
        gl.vertex_attrib_pointer_f32(self.thumb_a_uv, 2, glow::FLOAT, false, stride, 2 * std::mem::size_of::<f32>() as i32);

        gl.draw_arrays(glow::TRIANGLES, 0, 6);

        gl.disable_vertex_attrib_array(self.thumb_a_pos);
        gl.disable_vertex_attrib_array(self.thumb_a_uv);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.delete_buffer(vbo);
        gl.use_program(None);
    }

    // -----------------------------------------------------------------------
    // Galaxy view — shows all workspaces as planet icons
    // -----------------------------------------------------------------------

    pub fn draw_galaxy_view(&self, frame: &mut GlowFrame<'_, '_>, screen: Size<i32, Physical>,
                             orbital: &OrbitalSwitcher) -> anyhow::Result<()> {
        frame.with_context(|gl| unsafe { self.gl_draw_galaxy(&**gl, screen, orbital); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    /// DRM-backend variant.
    pub unsafe fn draw_galaxy_view_gl(&self, gl: &glow::Context, screen: Size<i32, Physical>, orbital: &OrbitalSwitcher) {
        self.gl_draw_galaxy(gl, screen, orbital);
    }

    unsafe fn gl_draw_galaxy(&self, gl: &glow::Context, screen: Size<i32, Physical>, orbital: &OrbitalSwitcher) {
        use crate::render::palette;
        gl.use_program(Some(self.geom_prog));
        gl.uniform_2_f32(Some(&self.geom_u_screen), screen.w as f32, screen.h as f32);
        gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        let cam = &orbital.camera;

        for (i, ws) in orbital.workspaces.iter().enumerate() {
            let sp = cam.world_to_screen().transform_point2(ws.world_pos);

            // Base radius scales with window count; active/hovered gets a boost.
            let base_r = (40.0 + ws.window_count() as f32 * 8.0) * cam.zoom;
            let is_active  = i == orbital.active;
            let is_hovered = orbital.hovered_ws == Some(i);
            let scale = if is_active || is_hovered { 1.35 } else { 1.0 };
            let r = base_r * scale;

            // Choose colour.
            let col = if is_active {
                palette::SUN_INNER
            } else if is_hovered {
                palette::PLANET_HOVER
            } else {
                palette::PLANET_BORDER
            };
            gl.uniform_4_f32(Some(&self.geom_u_color), col[0], col[1], col[2], col[3]);
            self.draw_circle_line(gl, sp.x, sp.y, r, 48);

            // Outer glow ring for active workspace.
            if is_active {
                let c = palette::SUN_OUTER;
                gl.uniform_4_f32(Some(&self.geom_u_color), c[0], c[1], c[2], c[3]);
                self.draw_circle_line(gl, sp.x, sp.y, r * 1.5, 48);
            }

            // Inner dot: small filled circle approximated via a tiny line loop.
            let dot_col = if is_active { palette::SUN_INNER } else { palette::PLANET_BORDER };
            gl.uniform_4_f32(Some(&self.geom_u_color), dot_col[0], dot_col[1], dot_col[2], dot_col[3]);
            self.draw_circle_line(gl, sp.x, sp.y, r * 0.25, 16);

            // Draw planets inside this workspace as tiny dots orbiting the ws icon.
            let p_col = palette::ORBIT_RING;
            gl.uniform_4_f32(Some(&self.geom_u_color), p_col[0], p_col[1], p_col[2], p_col[3]);
            let mini_orbit_r = r * 1.8;
            let n_planets = ws.planets.len();
            for (_j, planet) in ws.planets.iter().enumerate() {
                let angle = planet.angle;
                let px = sp.x + mini_orbit_r * angle.sin();
                let py = sp.y - mini_orbit_r * angle.cos();
                self.draw_circle_line(gl, px, py, r * 0.15, 8);
            }
            // Suppress unused variable warning in release builds.
            let _ = n_planets;
        }

        gl.disable(glow::BLEND); gl.use_program(None);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    unsafe fn draw_circle_line(&self, gl: &glow::Context, cx: f32, cy: f32, r: f32, seg: u32) {
        let v: Vec<f32> = (0..seg).flat_map(|i| {
            let a = std::f32::consts::TAU * i as f32 / seg as f32;
            [cx + r * a.cos(), cy + r * a.sin()]
        }).collect();
        let vbo = gl.create_buffer().expect("tmp");
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&v), glow::STREAM_DRAW);
        gl.enable_vertex_attrib_array(self.geom_a_pos);
        gl.vertex_attrib_pointer_f32(self.geom_a_pos, 2, glow::FLOAT, false, (2*std::mem::size_of::<f32>()) as i32, 0);
        gl.draw_arrays(glow::LINE_LOOP, 0, seg as i32);
        gl.disable_vertex_attrib_array(self.geom_a_pos);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.delete_buffer(vbo);
    }
}

// ---------------------------------------------------------------------------
// StarfieldElement — smithay RenderElement wrapper for the parallax starfield
// ---------------------------------------------------------------------------
//
// By pushing a `StarfieldElement` as the *last* entry in the elements slice
// passed to `render_output`, it ends up drawn first (render_output_internal
// iterates in reverse), placing stars visually below Wayland windows.

/// A lightweight handle that carries all GL state needed to draw one frame of
/// the parallax starfield. Created via `GlesSpaceRenderer::make_starfield_element`.
#[derive(Clone)]
pub struct StarfieldElement {
    id:     Id,
    commit: CommitCounter,
    // GL handles (all glow handle types are Copy)
    star_prog:      glow::Program,
    /// (vbo, vertex_count) for each of the 3 parallax layers
    star_layers:    [(glow::Buffer, i32); 3],
    star_a_pos:     u32,
    star_a_bright:  u32,
    star_a_phase:   u32,
    star_u_camera:  glow::UniformLocation,
    star_u_parallax: glow::UniformLocation,
    star_u_size:    glow::UniformLocation,
    star_u_time:    glow::UniformLocation,
    // Per-frame animation state (snapshotted at element creation time)
    cam_x:          f32,
    cam_y:          f32,
    time:           f32,
    layer_parallax: [f32; 3],
    layer_size:     [f32; 3],
    // Output dimensions
    width:          i32,
    height:         i32,
}

impl Element for StarfieldElement {
    fn id(&self) -> &Id { &self.id }

    fn current_commit(&self) -> CommitCounter { self.commit }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::new(
            (0.0_f64, 0.0_f64).into(),
            (self.width as f64, self.height as f64).into(),
        )
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        Rectangle::new(
            (0_i32, 0_i32).into(),
            (self.width, self.height).into(),
        )
    }
}

impl RenderElement<GlowRenderer> for StarfieldElement {
    fn draw(
        &self,
        frame: &mut GlowFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        _dst: Rectangle<i32, Physical>,
        _damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        frame.with_context(|gl: &std::sync::Arc<glow::Context>| unsafe {
            gl.use_program(Some(self.star_prog));
            gl.uniform_2_f32(Some(&self.star_u_camera), self.cam_x, self.cam_y);
            gl.uniform_1_f32(Some(&self.star_u_time), self.time);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE); // additive → glow effect

            let s        = (4 * std::mem::size_of::<f32>()) as i32;
            let f32_size = std::mem::size_of::<f32>() as i32;

            for (i, (vbo, count)) in self.star_layers.iter().enumerate() {
                gl.uniform_1_f32(Some(&self.star_u_parallax), self.layer_parallax[i]);
                gl.uniform_1_f32(Some(&self.star_u_size),     self.layer_size[i]);

                gl.bind_buffer(glow::ARRAY_BUFFER, Some(*vbo));
                gl.enable_vertex_attrib_array(self.star_a_pos);
                gl.vertex_attrib_pointer_f32(self.star_a_pos,    2, glow::FLOAT, false, s, 0);
                gl.enable_vertex_attrib_array(self.star_a_bright);
                gl.vertex_attrib_pointer_f32(self.star_a_bright, 1, glow::FLOAT, false, s, 2 * f32_size);
                gl.enable_vertex_attrib_array(self.star_a_phase);
                gl.vertex_attrib_pointer_f32(self.star_a_phase,  1, glow::FLOAT, false, s, 3 * f32_size);

                gl.draw_arrays(glow::POINTS, 0, *count);

                gl.disable_vertex_attrib_array(self.star_a_pos);
                gl.disable_vertex_attrib_array(self.star_a_bright);
                gl.disable_vertex_attrib_array(self.star_a_phase);
            }

            gl.disable(glow::BLEND);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.use_program(None);
        })
    }
}
