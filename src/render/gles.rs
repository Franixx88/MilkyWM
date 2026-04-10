use glow::HasContext;
use smithay::backend::renderer::glow::GlowRenderer;
use smithay::utils::{Physical, Size};
use tracing::debug;

use crate::{orbital::OrbitalSwitcher, render::space::Starfield};

const STAR_VERT: &str = r#"
    attribute vec2  a_pos;
    attribute float a_brightness;
    varying   float v_brightness;
    uniform vec2 u_camera_offset;
    void main() {
        vec2 uv  = fract(a_pos - u_camera_offset * 0.003);
        vec2 ndc = uv * 2.0 - 1.0;
        ndc.y    = -ndc.y;
        gl_Position  = vec4(ndc, 0.0, 1.0);
        gl_PointSize = mix(1.0, 2.5, a_brightness);
        v_brightness = a_brightness;
    }
"#;

const STAR_FRAG: &str = r#"
    precision mediump float;
    varying float v_brightness;
    void main() {
        float d     = length(gl_PointCoord - vec2(0.5));
        float alpha = (1.0 - smoothstep(0.3, 0.5, d)) * v_brightness;
        gl_FragColor = vec4(0.85, 0.90, 1.00, alpha);
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

pub struct GlesSpaceRenderer {
    star_prog:     glow::Program,
    star_vbo:      glow::Buffer,
    star_count:    i32,
    star_a_pos:    u32,
    star_a_bright: u32,
    star_u_camera: glow::UniformLocation,
    geom_prog:     glow::Program,
    geom_a_pos:    u32,
    geom_u_screen: glow::UniformLocation,
    geom_u_color:  glow::UniformLocation,
}

impl GlesSpaceRenderer {
    pub fn init(renderer: &mut GlowRenderer, starfield: &Starfield) -> anyhow::Result<Self> {
        let mut out: Option<anyhow::Result<Self>> = None;
        renderer.with_context(|gl| { out = Some(unsafe { Self::init_gl(gl, starfield) }); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        out.unwrap()
    }

    unsafe fn init_gl(gl: &glow::Context, starfield: &Starfield) -> anyhow::Result<Self> {
        let star_prog     = compile_program(gl, STAR_VERT, STAR_FRAG)?;
        let star_a_pos    = gl.get_attrib_location(star_prog, "a_pos").ok_or_else(|| anyhow::anyhow!("a_pos"))? as u32;
        let star_a_bright = gl.get_attrib_location(star_prog, "a_brightness").ok_or_else(|| anyhow::anyhow!("a_brightness"))? as u32;
        let star_u_camera = gl.get_uniform_location(star_prog, "u_camera_offset").ok_or_else(|| anyhow::anyhow!("u_camera_offset"))?;

        let star_vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(star_vbo));
        let mut data: Vec<f32> = Vec::with_capacity(starfield.stars.len() * 3);
        for s in &starfield.stars { data.push(s.pos.x); data.push(s.pos.y); data.push(s.brightness); }
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(&data), glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        let star_count = starfield.stars.len() as i32;

        let geom_prog     = compile_program(gl, GEOM_VERT, GEOM_FRAG)?;
        let geom_a_pos    = gl.get_attrib_location(geom_prog, "a_pos").ok_or_else(|| anyhow::anyhow!("geom a_pos"))? as u32;
        let geom_u_screen = gl.get_uniform_location(geom_prog, "u_screen_size").ok_or_else(|| anyhow::anyhow!("geom u_screen_size"))?;
        let geom_u_color  = gl.get_uniform_location(geom_prog, "u_color").ok_or_else(|| anyhow::anyhow!("geom u_color"))?;

        debug!("GlesSpaceRenderer ready — {} stars", star_count);
        Ok(Self { star_prog, star_vbo, star_count, star_a_pos, star_a_bright, star_u_camera,
                  geom_prog, geom_a_pos, geom_u_screen, geom_u_color })
    }

    pub fn draw_starfield(&self, renderer: &mut GlowRenderer, _screen: Size<i32, Physical>,
                          _starfield: &Starfield, cx: f32, cy: f32) -> anyhow::Result<()> {
        renderer.with_context(|gl| unsafe {
            gl.clear_color(0.0, 0.0, 0.03, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.star_prog));
            gl.uniform_2_f32(Some(&self.star_u_camera), cx, cy);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.star_vbo));
            let s = (3 * std::mem::size_of::<f32>()) as i32;
            gl.enable_vertex_attrib_array(self.star_a_pos);
            gl.vertex_attrib_pointer_f32(self.star_a_pos, 2, glow::FLOAT, false, s, 0);
            gl.enable_vertex_attrib_array(self.star_a_bright);
            gl.vertex_attrib_pointer_f32(self.star_a_bright, 1, glow::FLOAT, false, s, 2 * std::mem::size_of::<f32>() as i32);
            gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE);
            gl.draw_arrays(glow::POINTS, 0, self.star_count);
            gl.disable(glow::BLEND);
            gl.disable_vertex_attrib_array(self.star_a_pos);
            gl.disable_vertex_attrib_array(self.star_a_bright);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.use_program(None);
        }).map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    // -----------------------------------------------------------------------
    // System view — orbital overlay for the active workspace
    // -----------------------------------------------------------------------

    pub fn draw_orbital_overlay(&self, renderer: &mut GlowRenderer, screen: Size<i32, Physical>,
                                 orbital: &OrbitalSwitcher) -> anyhow::Result<()> {
        renderer.with_context(|gl| unsafe { self.gl_draw_orbital(gl, screen, orbital); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    unsafe fn gl_draw_orbital(&self, gl: &glow::Context, screen: Size<i32, Physical>, orbital: &OrbitalSwitcher) {
        use crate::render::palette;
        gl.use_program(Some(self.geom_prog));
        gl.uniform_2_f32(Some(&self.geom_u_screen), screen.w as f32, screen.h as f32);
        gl.enable(glow::BLEND); gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        let cam = &orbital.camera;
        let ws = orbital.active_ws();

        // Orbit rings
        let p = palette::ORBIT_RING;
        gl.uniform_4_f32(Some(&self.geom_u_color), p[0], p[1], p[2], p[3]);
        let max_ring = ws.planets.iter().map(|p| p.orbit_index).max().map(|m| m + 1).unwrap_or(0);
        for ring in 0..max_ring {
            let r = (crate::orbital::body::ORBIT_BASE_RADIUS + ring as f32 * crate::orbital::body::ORBIT_STEP) * cam.zoom;
            let c = cam.world_to_screen().transform_point2(ws.world_pos);
            self.draw_circle_line(gl, c.x, c.y, r, 64);
        }
        // Planet halos
        for (i, planet) in ws.planets.iter().enumerate() {
            let planet_world = ws.world_pos + planet.world_pos();
            let sp  = cam.world_to_screen().transform_point2(planet_world);
            let r   = planet.visual_diameter() * 0.5 * cam.zoom;
            let col = if orbital.hovered_planet == Some(i) { palette::PLANET_HOVER } else { palette::PLANET_BORDER };
            gl.uniform_4_f32(Some(&self.geom_u_color), col[0], col[1], col[2], col[3] * planet.alpha);
            self.draw_circle_line(gl, sp.x, sp.y, r, 32);
        }
        // Sun corona
        if ws.sun.is_some() {
            let c    = cam.world_to_screen().transform_point2(ws.world_pos);
            let base = 80.0 * cam.zoom;
            for (f, col) in &[(1.0_f32, palette::SUN_INNER), (1.4_f32, palette::SUN_OUTER)] {
                gl.uniform_4_f32(Some(&self.geom_u_color), col[0], col[1], col[2], col[3]);
                self.draw_circle_line(gl, c.x, c.y, base * f, 64);
            }
        }
        gl.disable(glow::BLEND); gl.use_program(None);
    }

    // -----------------------------------------------------------------------
    // Galaxy view — shows all workspaces as planet icons
    // -----------------------------------------------------------------------

    pub fn draw_galaxy_view(&self, renderer: &mut GlowRenderer, screen: Size<i32, Physical>,
                             orbital: &OrbitalSwitcher) -> anyhow::Result<()> {
        renderer.with_context(|gl| unsafe { self.gl_draw_galaxy(gl, screen, orbital); })
            .map_err(|e| anyhow::anyhow!("{e:?}"))
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
