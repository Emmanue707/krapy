use crate::overlay::OverlayQuad;
use krapy_decode::{Frame, YuvRgb, yuv_rgb_coeffs};
use std::mem;
use std::sync::Arc;
use winit::window::Window;

const OVERLAY_WGSL: &str = r"
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) col: vec4f,
};
@vertex
fn vs_main(@location(0) p: vec2f, @location(1) col: vec4f) -> VsOut {
    var o: VsOut;
    o.pos = vec4f(p, 0.0, 1.0);
    o.col = col;
    return o;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return in.col;
}
";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OverlayVert {
    pos: [f32; 2],
    col: [f32; 4],
}

const SHADER: &str = r"
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

struct ColorUni {
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    y_scale: f32,
    y_bias: f32,
    u_scale: f32,
    u_bias: f32,
    v_scale: f32,
    v_bias: f32,
};

struct Params {
    tex_size: vec2f,
    scale: f32,
    rot: u32,
    out_srgb: u32,
};

@group(0) @binding(0) var t_y: texture_2d<f32>;
@group(0) @binding(1) var t_u: texture_2d<f32>;
@group(0) @binding(2) var t_v: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;
@group(0) @binding(4) var<uniform> color: ColorUni;
@group(0) @binding(5) var<uniform> params: Params;

fn rot_uv(uv: vec2f, rot: u32) -> vec2f {
    switch rot {
        case 1u: { return vec2f(uv.y, 1.0 - uv.x); }
        case 2u: { return vec2f(1.0 - uv.x, 1.0 - uv.y); }
        case 3u: { return vec2f(1.0 - uv.y, uv.x); }
        default: { return uv; }
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    out.pos = vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = rot_uv(vec2f(x, y), params.rot);
    return out;
}

fn rgb_from_yuv(y: f32, u: f32, v: f32) -> vec3f {
    let yn = y * color.y_scale + color.y_bias;
    let un = u * color.u_scale + color.u_bias;
    let vn = v * color.v_scale + color.v_bias;
    return vec3f(
        dot(color.m0.xyz, vec3f(yn, un, vn)),
        dot(color.m1.xyz, vec3f(yn, un, vn)),
        dot(color.m2.xyz, vec3f(yn, un, vn)),
    );
}

fn sample_y_bilinear(uv: vec2f) -> f32 {
    return textureSampleLevel(t_y, smp, uv, 0.0).r * 255.0;
}

fn sample_y_area(uv: vec2f) -> f32 {
    let dx = 1.0 / (params.tex_size.x * params.scale);
    let dy = 1.0 / (params.tex_size.y * params.scale);
    let hdx = dx * 0.5;
    let hdy = dy * 0.5;
    return (
        sample_y_bilinear(uv + vec2f(-hdx, -hdy)) +
        sample_y_bilinear(uv + vec2f(hdx, -hdy)) +
        sample_y_bilinear(uv + vec2f(-hdx, hdy)) +
        sample_y_bilinear(uv + vec2f(hdx, hdy))
    ) * 0.25;
}

fn catmull_w(d: f32) -> vec4f {
    let d2 = d * d;
    let d3 = d2 * d;
    return vec4f(
        -0.5 * d3 + d2 - 0.5 * d,
        1.5 * d3 - 2.5 * d2 + 1.0,
        -1.5 * d3 + 2.0 * d2 + 0.5 * d,
        0.5 * d3 - 0.5 * d2,
    );
}

fn sample_y_catmull(uv: vec2f) -> f32 {
    let st = uv * params.tex_size - 0.5;
    let i = floor(st);
    let f = st - i;
    let wx = catmull_w(f.x);
    let wy = catmull_w(f.y);
    var acc = 0.0;
    for (var y = 0; y < 4; y++) {
        let uv_y = (i.y + f32(y) - 1.0 + 0.5) / params.tex_size.y;
        for (var x = 0; x < 4; x++) {
            let uv_x = (i.x + f32(x) - 1.0 + 0.5) / params.tex_size.x;
            acc += sample_y_bilinear(vec2f(uv_x, uv_y)) * wx[x] * wy[y];
        }
    }
    return acc;
}

fn sample_y(uv: vec2f) -> f32 {
    if params.scale < 0.999 {
        return sample_y_area(uv);
    }
    if params.scale > 1.001 {
        return sample_y_catmull(uv);
    }
    return sample_y_bilinear(uv);
}

fn srgb_to_linear(c: vec3f) -> vec3f {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(low, high, c > vec3f(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let uv_y = in.uv;
    let y = sample_y(uv_y);
    let u = textureSampleLevel(t_u, smp, uv_y, 0.0).r * 255.0;
    let v = textureSampleLevel(t_v, smp, uv_y, 0.0).r * 255.0;
    var rgb = rgb_from_yuv(y, u, v);
    if params.out_srgb != 0u {
        rgb = srgb_to_linear(rgb);
    }
    return vec4f(rgb, 1.0);
}
";

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ColorUniform {
    m0: [f32; 4],
    m1: [f32; 4],
    m2: [f32; 4],
    y_scale: f32,
    y_bias: f32,
    u_scale: f32,
    u_bias: f32,
    v_scale: f32,
    v_bias: f32,
    _pad: [u32; 2],
}

const _: () = assert!(mem::size_of::<ColorUniform>() == 80);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ParamsUniform {
    tex_size: [f32; 2],
    scale: f32,
    rot: u32,
    out_srgb: u32,
    _pad: u32,
}

const _: () = assert!(mem::size_of::<ParamsUniform>() == 24);

pub(crate) fn write_params(
    queue: &wgpu::Queue,
    buf: &wgpu::Buffer,
    frame_w: u32,
    frame_h: u32,
    display_scale: f32,
    rot: u8,
    out_srgb: bool,
) {
    let params = ParamsUniform {
        tex_size: [frame_w as f32, frame_h as f32],
        scale: display_scale,
        rot: rot as u32,
        out_srgb: u32::from(out_srgb),
        _pad: 0,
    };
    queue.write_buffer(buf, 0, bytemuck::bytes_of(&params));
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    color_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    y_tex: Option<wgpu::Texture>,
    u_tex: Option<wgpu::Texture>,
    v_tex: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    tex_size: (u32, u32),
    surface_srgb: bool,
    queued_color: Option<krapy_decode::ColorSpec>,
    overlay_pipeline: Option<wgpu::RenderPipeline>,
    overlay_vbuf: Option<wgpu::Buffer>,
    overlay_verts: u32,
}

pub(crate) struct YuvGfx {
    pub bind_layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub sampler: wgpu::Sampler,
    pub color_buf: wgpu::Buffer,
    pub params_buf: wgpu::Buffer,
}

pub(crate) fn create_yuv_gfx(device: &wgpu::Device, format: wgpu::TextureFormat) -> YuvGfx {
    let bind_layout = create_bind_group_layout(device);
    let pipeline = create_yuv_pipeline(device, &bind_layout, format);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let color_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("color"),
        size: mem::size_of::<ColorUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: mem::size_of::<ParamsUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    YuvGfx {
        bind_layout,
        pipeline,
        sampler,
        color_buf,
        params_buf,
    }
}

impl Gpu {
    pub fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("krapy"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
            }))?;
        let caps = surface.get_capabilities(&adapter);
        let (format, surface_srgb) = pick_format(&caps.formats);
        let present = pick_present(&caps.present_modes);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: present,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);
        let YuvGfx {
            bind_layout,
            pipeline,
            sampler,
            color_buf,
            params_buf,
        } = create_yuv_gfx(&device, format);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_layout,
            sampler,
            color_buf,
            params_buf,
            y_tex: None,
            u_tex: None,
            v_tex: None,
            bind_group: None,
            tex_size: (0, 0),
            surface_srgb,
            queued_color: None,
            overlay_pipeline: None,
            overlay_vbuf: None,
            overlay_verts: 0,
        })
    }

    fn ensure_overlay(&mut self) {
        if self.overlay_pipeline.is_some() {
            return;
        }
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("overlay"),
                source: wgpu::ShaderSource::Wgsl(OVERLAY_WGSL.into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("overlay"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("overlay"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: mem::size_of::<OverlayVert>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 0,
                                format: wgpu::VertexFormat::Float32x2,
                            },
                            wgpu::VertexAttribute {
                                offset: 8,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32x4,
                            },
                        ],
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });
        self.overlay_pipeline = Some(pipeline);
    }

    pub fn set_overlay_quads(&mut self, quads: &[OverlayQuad], screen_w: f32, screen_h: f32) {
        if quads.is_empty() {
            self.overlay_verts = 0;
            return;
        }
        self.ensure_overlay();
        let mut v = Vec::with_capacity(quads.len() * 6);
        for q in quads {
            let rgba = [q.r, q.g, q.b, q.a];
            let x0 = (q.x / screen_w) * 2.0 - 1.0;
            let x1 = ((q.x + q.w) / screen_w) * 2.0 - 1.0;
            let y0 = 1.0 - (q.y / screen_h) * 2.0;
            let y1 = 1.0 - ((q.y + q.h) / screen_h) * 2.0;
            v.push(OverlayVert {
                pos: [x0, y0],
                col: rgba,
            });
            v.push(OverlayVert {
                pos: [x1, y0],
                col: rgba,
            });
            v.push(OverlayVert {
                pos: [x0, y1],
                col: rgba,
            });
            v.push(OverlayVert {
                pos: [x1, y0],
                col: rgba,
            });
            v.push(OverlayVert {
                pos: [x1, y1],
                col: rgba,
            });
            v.push(OverlayVert {
                pos: [x0, y1],
                col: rgba,
            });
        }
        let need = (v.len() * mem::size_of::<OverlayVert>()) as u64;
        if self.overlay_vbuf.as_ref().is_none_or(|b| b.size() < need) {
            let size = need.next_power_of_two().max(256);
            self.overlay_vbuf = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlay"),
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buf = self.overlay_vbuf.as_ref().expect("overlay buf");
        self.queue.write_buffer(buf, 0, bytemuck::cast_slice(&v));
        self.overlay_verts = v.len() as u32;
    }

    pub fn clear_overlay(&mut self) {
        self.overlay_verts = 0;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    fn ensure_textures(&mut self, frame: &Frame) {
        let need = frame.width != self.tex_size.0 || frame.height != self.tex_size.1;
        if !need {
            return;
        }
        let mk = |label: &str, w: u32, h: u32| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        self.y_tex = Some(mk("y", frame.width, frame.height));
        let hw = frame.width / 2;
        let hh = frame.height / 2;
        self.u_tex = Some(mk("u", hw, hh));
        self.v_tex = Some(mk("v", hw, hh));
        self.tex_size = (frame.width, frame.height);
        self.bind_group = None;
    }

    fn ensure_bind_group(&mut self) {
        if self.bind_group.is_some() {
            return;
        }
        let y = self.y_tex.as_ref().expect("y");
        let u = self.u_tex.as_ref().expect("u");
        let v = self.v_tex.as_ref().expect("v");
        let yv = y.create_view(&Default::default());
        let uv = u.create_view(&Default::default());
        let vv = v.create_view(&Default::default());
        self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&yv),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&vv),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.color_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.params_buf.as_entire_binding(),
                },
            ],
        }));
    }

    fn write_color(&mut self, spec: krapy_decode::ColorSpec) {
        if self.queued_color == Some(spec) {
            return;
        }
        self.queued_color = Some(spec);
        let c = yuv_rgb_coeffs(spec);
        let u = color_to_uniform(&c);
        self.queue
            .write_buffer(&self.color_buf, 0, bytemuck::bytes_of(&u));
    }

    pub fn upload(&mut self, frame: &Frame) {
        self.ensure_textures(frame);
        self.write_color(frame.color);
        let y = self.y_tex.as_ref().expect("y");
        let u = self.u_tex.as_ref().expect("u");
        let v = self.v_tex.as_ref().expect("v");
        upload_plane(
            &self.queue,
            y,
            &frame.y,
            frame.stride_y,
            frame.width,
            frame.height,
        );
        let hw = frame.width / 2;
        let hh = frame.height / 2;
        upload_plane(&self.queue, u, &frame.u, frame.stride_uv, hw, hh);
        upload_plane(&self.queue, v, &frame.v, frame.stride_uv, hw, hh);
        self.ensure_bind_group();
    }

    pub fn draw(
        &mut self,
        viewport: crate::letterbox::Viewport,
        frame_w: u32,
        frame_h: u32,
        rot: u8,
        display_scale: f32,
    ) -> anyhow::Result<()> {
        let bind = self
            .bind_group
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no frame"))?;
        write_params(
            &self.queue,
            &self.params_buf,
            frame_w,
            frame_h,
            display_scale,
            rot,
            self.surface_srgb,
        );
        let surface_frame = self.surface.get_current_texture()?;
        let view = surface_frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("draw"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_viewport(viewport.x, viewport.y, viewport.w, viewport.h, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
            if self.overlay_verts > 0 {
                let pip = self.overlay_pipeline.as_ref().expect("overlay");
                let vbuf = self.overlay_vbuf.as_ref().expect("overlay buf");
                pass.set_pipeline(pip);
                pass.set_viewport(
                    0.0,
                    0.0,
                    self.config.width as f32,
                    self.config.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_vertex_buffer(0, vbuf.slice(..));
                pass.draw(0..self.overlay_verts, 0..1);
            }
        }
        self.queue.submit(Some(enc.finish()));
        surface_frame.present();
        Ok(())
    }
}

pub(crate) fn color_to_uniform(c: &YuvRgb) -> ColorUniform {
    ColorUniform {
        m0: [c.m[0][0], c.m[0][1], c.m[0][2], 0.0],
        m1: [c.m[1][0], c.m[1][1], c.m[1][2], 0.0],
        m2: [c.m[2][0], c.m[2][1], c.m[2][2], 0.0],
        y_scale: c.y_scale,
        y_bias: c.y_bias,
        u_scale: c.u_scale,
        u_bias: c.u_bias,
        v_scale: c.v_scale,
        v_bias: c.v_bias,
        _pad: [0, 0],
    }
}

fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("yuv"),
        entries: &[
            tex_entry(0, wgpu::ShaderStages::FRAGMENT),
            tex_entry(1, wgpu::ShaderStages::FRAGMENT),
            tex_entry(2, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            uniform_entry(4, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
            uniform_entry(5, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

fn create_yuv_pipeline(
    device: &wgpu::Device,
    bind_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("yuv"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("yuv"),
        bind_group_layouts: &[bind_layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("yuv"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn tex_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(crate) fn upload_plane(
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    data: &[u8],
    stride: usize,
    width: u32,
    height: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn pick_format(formats: &[wgpu::TextureFormat]) -> (wgpu::TextureFormat, bool) {
    for want in [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        if formats.contains(&want) {
            return (want, false);
        }
    }
    if let Some(f) = formats.iter().copied().find(|f| !f.is_srgb()) {
        return (f, false);
    }
    let srgb = formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .or_else(|| formats.first().copied())
        .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);
    (srgb, true)
}

fn pick_present(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else if modes.contains(&wgpu::PresentMode::Immediate) {
        wgpu::PresentMode::Immediate
    } else if modes.contains(&wgpu::PresentMode::Fifo) {
        wgpu::PresentMode::Fifo
    } else {
        wgpu::PresentMode::AutoVsync
    }
}

#[cfg(test)]
#[path = "gpu_offscreen.rs"]
mod gpu_offscreen;

pub fn probe_gpu() -> Option<String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    let info = adapter.get_info();
    Some(format!("{:?}/{}", info.backend, info.name))
}
