use super::{YuvGfx, color_to_uniform, create_yuv_gfx, upload_plane, write_params};
use crate::letterbox::window_to_video;
use krapy_decode::{ColorSpec, Frame, Matrix, yuv_rgb_coeffs, yuv_to_rgb};

struct GpuTestCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn try_ctx() -> Option<GpuTestCtx> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: true,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("krapy-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    println!(
        "gpu offscreen: {:?}/{}",
        adapter.get_info().backend,
        adapter.get_info().name
    );
    Some(GpuTestCtx { device, queue })
}

fn solid_frame(w: u32, h: u32, y: u8, u: u8, v: u8, color: ColorSpec) -> Frame {
    let uv_len = (w / 2 * h / 2) as usize;
    Frame {
        width: w,
        height: h,
        stride_y: w as usize,
        stride_uv: (w / 2) as usize,
        color,
        y: vec![y; (w * h) as usize],
        u: vec![u; uv_len],
        v: vec![v; uv_len],
    }
}

fn quad_frame(w: u32, h: u32) -> Frame {
    let hw = w / 2;
    let hh = h / 2;
    let mut frame = Frame {
        width: w,
        height: h,
        stride_y: w as usize,
        stride_uv: (w / 2) as usize,
        color: ColorSpec::DEFAULT,
        y: vec![16; (w * h) as usize],
        u: vec![128; (hw * hh) as usize],
        v: vec![128; (hw * hh) as usize],
    };
    for row in 0..h {
        for col in 0..w {
            let y = match (col < hw, row < hh) {
                (true, true) => 81u8,
                (false, true) => 41,
                (true, false) => 112,
                (false, false) => 235,
            };
            frame.y[(row * w + col) as usize] = y;
        }
    }
    for row in 0..hh {
        for col in 0..hw {
            let (u, v) = match (col < hw / 2, row < hh / 2) {
                (true, true) => (90u8, 240),
                (false, true) => (240, 110),
                (true, false) => (255, 74),
                (false, false) => (128, 128),
            };
            frame.u[(row * hw + col) as usize] = u;
            frame.v[(row * hw + col) as usize] = v;
        }
    }
    frame
}

fn read_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let bytes_per_pixel = 4u32;
    let unpadded = bytes_per_pixel * width;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row = unpadded.div_ceil(align) * align;
    let size = bytes_per_row as u64 * height as u64;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback"),
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::Wait);
    rx.recv().expect("map").expect("map ok");
    let mapped = slice.get_mapped_range();
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * bytes_per_row) as usize;
        out.extend_from_slice(&mapped[start..start + (unpadded as usize)]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

fn frame_textures(
    device: &wgpu::Device,
    frame: &Frame,
) -> (wgpu::Texture, wgpu::Texture, wgpu::Texture) {
    let mk = |w: u32, h: u32| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("plane"),
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
    (
        mk(frame.width, frame.height),
        mk(frame.width / 2, frame.height / 2),
        mk(frame.width / 2, frame.height / 2),
    )
}

fn upload_frame(
    queue: &wgpu::Queue,
    frame: &Frame,
    y: &wgpu::Texture,
    u: &wgpu::Texture,
    v: &wgpu::Texture,
) {
    upload_plane(
        queue,
        y,
        &frame.y,
        frame.stride_y,
        frame.width,
        frame.height,
    );
    let hw = frame.width / 2;
    let hh = frame.height / 2;
    upload_plane(queue, u, &frame.u, frame.stride_uv, hw, hh);
    upload_plane(queue, v, &frame.v, frame.stride_uv, hw, hh);
}

#[allow(clippy::too_many_arguments)]
fn render(
    ctx: &GpuTestCtx,
    gfx: &YuvGfx,
    frame: &Frame,
    out_w: u32,
    out_h: u32,
    format: wgpu::TextureFormat,
    rot: u8,
    display_scale: f32,
    out_srgb: bool,
) -> Vec<u8> {
    let (y_tex, u_tex, v_tex) = frame_textures(&ctx.device, frame);
    upload_frame(&ctx.queue, frame, &y_tex, &u_tex, &v_tex);
    let c = yuv_rgb_coeffs(frame.color);
    ctx.queue
        .write_buffer(&gfx.color_buf, 0, bytemuck::bytes_of(&color_to_uniform(&c)));
    write_params(
        &ctx.queue,
        &gfx.params_buf,
        frame.width,
        frame.height,
        display_scale,
        rot,
        out_srgb,
    );
    let yv = y_tex.create_view(&Default::default());
    let uv = u_tex.create_view(&Default::default());
    let vv = v_tex.create_view(&Default::default());
    let bind = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("yuv"),
        layout: &gfx.bind_layout,
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
                resource: wgpu::BindingResource::Sampler(&gfx.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: gfx.color_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: gfx.params_buf.as_entire_binding(),
            },
        ],
    });
    let out_tex = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("out"),
        size: wgpu::Extent3d {
            width: out_w,
            height: out_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = out_tex.create_view(&Default::default());
    let mut enc = ctx
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
        pass.set_viewport(0.0, 0.0, out_w as f32, out_h as f32, 0.0, 1.0);
        pass.set_pipeline(&gfx.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    ctx.queue.submit(Some(enc.finish()));
    read_rgba(&ctx.device, &ctx.queue, &out_tex, out_w, out_h)
}

fn px_rgb(px: &[u8], x: u32, y: u32, w: u32) -> (u8, u8, u8) {
    let i = ((y * w + x) * 4) as usize;
    (px[i], px[i + 1], px[i + 2])
}

fn near(a: u8, b: u8) -> bool {
    (i16::from(a) - i16::from(b)).unsigned_abs() <= 2
}

fn near_rgb(px: &[u8], x: u32, y: u32, w: u32, er: u8, eg: u8, eb: u8) {
    let (r, g, b) = px_rgb(px, x, y, w);
    assert!(
        near(r, er) && near(g, eg) && near(b, eb),
        "({x},{y}): got ({r},{g},{b}) want ({er},{eg},{eb})"
    );
}

fn linear_byte(spec: ColorSpec, y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let (rf, gf, bf) = yuv_to_rgb(spec, y, u, v);
    (
        (rf.clamp(0.0, 1.0) * 255.0).round() as u8,
        (gf.clamp(0.0, 1.0) * 255.0).round() as u8,
        (bf.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[test]
fn gpu_offscreen_render_pipeline() {
    let Some(ctx) = try_ctx() else {
        println!("gpu offscreen: no adapter, skip");
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let gfx = create_yuv_gfx(&ctx.device, format);
    let spec = ColorSpec::DEFAULT;
    let w = 16u32;
    let h = 16u32;

    let white = solid_frame(w, h, 235, 128, 128, spec);
    let px = render(&ctx, &gfx, &white, w, h, format, 0, 1.0, false);
    near_rgb(&px, w / 2, h / 2, w, 255, 255, 255);

    let black = solid_frame(w, h, 16, 128, 128, spec);
    let px = render(&ctx, &gfx, &black, w, h, format, 0, 1.0, false);
    near_rgb(&px, w / 2, h / 2, w, 0, 0, 0);

    let red = solid_frame(w, h, 81, 90, 240, spec);
    let px = render(&ctx, &gfx, &red, w, h, format, 0, 1.0, false);
    let (er, eg, eb) = linear_byte(spec, 81, 90, 240);
    near_rgb(&px, w / 2, h / 2, w, er, eg, eb);
    let (r, g, b) = px_rgb(&px, w / 2, h / 2, w);
    assert!(r >= 253 && g <= 26 && b <= 2);

    let grey = solid_frame(w, h, 126, 128, 128, spec);
    let px = render(&ctx, &gfx, &grey, w, h, format, 0, 1.0, false);
    let (gr, gg, gb) = linear_byte(spec, 126, 128, 128);
    near_rgb(&px, w / 2, h / 2, w, gr, gg, gb);
    assert!((i16::from(gr) - 128).unsigned_abs() <= 2);

    let spec601 = ColorSpec {
        full_range: true,
        matrix: Matrix::Bt601,
    };
    let hi = solid_frame(w, h, 255, 128, 128, spec601);
    let px = render(&ctx, &gfx, &hi, w, h, format, 0, 1.0, false);
    near_rgb(&px, w / 2, h / 2, w, 255, 255, 255);
    let lo = solid_frame(w, h, 0, 128, 128, spec601);
    let px = render(&ctx, &gfx, &lo, w, h, format, 0, 1.0, false);
    near_rgb(&px, w / 2, h / 2, w, 0, 0, 0);

    let lin = render(
        &ctx,
        &gfx,
        &grey,
        w,
        h,
        wgpu::TextureFormat::Rgba8Unorm,
        0,
        1.0,
        false,
    );
    let srgb_fmt = wgpu::TextureFormat::Rgba8UnormSrgb;
    let gfx_srgb = create_yuv_gfx(&ctx.device, srgb_fmt);
    let srgb = render(&ctx, &gfx_srgb, &grey, w, h, srgb_fmt, 0, 1.0, true);
    assert_eq!(lin, srgb, "srgb path must not lift stored bytes");

    let qf = quad_frame(8, 8);
    let px = render(&ctx, &gfx, &qf, 8, 8, format, 1, 1.0, false);
    let red = linear_byte(spec, 81, 90, 240);
    let green = linear_byte(spec, 41, 240, 110);
    let blue = linear_byte(spec, 112, 255, 74);
    let white = linear_byte(spec, 235, 128, 128);
    for (wx, wy) in [(1u32, 1), (7, 1), (1, 7), (7, 7)] {
        let (vx, vy) = window_to_video(wx as f32, wy as f32, 8.0, 8.0, 8, 8, 1).expect("map");
        let (er, eg, eb) = if vx <= 3 && vy <= 3 {
            red
        } else if vx >= 4 && vy <= 3 {
            green
        } else if vx <= 3 && vy >= 4 {
            blue
        } else {
            white
        };
        near_rgb(&px, wx, wy, 8, er, eg, eb);
    }

    let flat = solid_frame(32, 32, 81, 90, 240, spec);
    let (er, eg, eb) = linear_byte(spec, 81, 90, 240);
    let px = render(&ctx, &gfx, &flat, 16, 16, format, 0, 0.4, false);
    near_rgb(&px, 8, 8, 16, er, eg, eb);
    let px = render(&ctx, &gfx, &flat, 32, 32, format, 0, 2.0, false);
    near_rgb(&px, 16, 16, 32, er, eg, eb);
}
