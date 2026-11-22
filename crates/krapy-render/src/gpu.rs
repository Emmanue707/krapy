use krapy_decode::Frame;
use std::sync::Arc;
use winit::window::Window;

const SHADER: &str = r"
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    out.pos = vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2f(x, y);
    return out;
}

@group(0) @binding(0) var t_y: texture_2d<f32>;
@group(0) @binding(1) var t_u: texture_2d<f32>;
@group(0) @binding(2) var t_v: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let y = textureSample(t_y, smp, in.uv).r;
    let u = textureSample(t_u, smp, in.uv).r;
    let v = textureSample(t_v, smp, in.uv).r;
    let yf = (y * 255.0 - 16.0) / 219.0;
    let uf = (u * 255.0 - 128.0) / 224.0;
    let vf = (v * 255.0 - 128.0) / 224.0;
    let r = yf + 1.402 * vf;
    let g = yf - 0.344136 * uf - 0.714136 * vf;
    let b = yf + 1.772 * uf;
    return vec4f(r, g, b, 1.0);
}
";

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    y_tex: Option<wgpu::Texture>,
    u_tex: Option<wgpu::Texture>,
    v_tex: Option<wgpu::Texture>,
    tex_size: (u32, u32),
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("krapy"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
            trace: wgpu::Trace::Off,
        }))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
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
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        });
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_layout,
            sampler,
            y_tex: None,
            u_tex: None,
            v_tex: None,
            tex_size: (0, 0),
        })
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
    }

    pub fn upload(&mut self, frame: &Frame) {
        self.ensure_textures(frame);
        let y = self.y_tex.as_ref().expect("y");
        let u = self.u_tex.as_ref().expect("u");
        let v = self.v_tex.as_ref().expect("v");
        upload_plane(&self.queue, y, &frame.y, frame.stride_y, frame.width, frame.height);
        let hw = frame.width / 2;
        let hh = frame.height / 2;
        upload_plane(&self.queue, u, &frame.u, frame.stride_uv, hw, hh);
        upload_plane(&self.queue, v, &frame.v, frame.stride_uv, hw, hh);
    }

    pub fn draw(&mut self, viewport: crate::letterbox::Viewport) -> anyhow::Result<()> {
        let y = self.y_tex.as_ref().ok_or_else(|| anyhow::anyhow!("no frame"))?;
        let u = self.u_tex.as_ref().expect("u");
        let v = self.v_tex.as_ref().expect("v");
        let yv = y.create_view(&Default::default());
        let uv = u.create_view(&Default::default());
        let vv = v.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            ],
        });
        let frame = self.surface.get_current_texture()?;
        let view = frame
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
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        frame.present();
        Ok(())
    }
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

fn upload_plane(
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

fn pick_present(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::AutoVsync
    }
}