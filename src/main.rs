use raw_window_handle::HasWindowHandle;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use crate::device_interface::DeviceInterface;
use crate::pipeline::Pipeline;
use crate::scene::Scene;
use crate::surface::Surface;
use crate::window_handle::WindowHandle;

mod device_interface;
mod imports;
mod pipeline;
mod resource;
mod scene;
mod surface;
mod window_handle;

fn render(
    interface: &DeviceInterface,
    scene: &mut Scene,
    pipeline: &Pipeline,
    surface: &Surface,
) -> windows::core::Result<()> {
    scene.update(&interface);

    pipeline.bind(&interface);
    scene.bind(&interface);
    let surface_desc = surface.bind(&interface)?;
    let rays_desc = pipeline.create_rays_description(&surface_desc);
    unsafe { interface.command_list.DispatchRays(&rays_desc) };

    surface.present(&interface)?;
    interface.wait_for_gpu()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new().unwrap();
    let window = WindowBuilder::new()
        .with_title("DirectX 11")
        .build(&event_loop)
        .unwrap();

    let window_handle: WindowHandle = window.window_handle()?.as_raw().try_into()?;
    let window_handle = window_handle.into();
    event_loop.set_control_flow(ControlFlow::Poll);

    let interface = DeviceInterface::create()?;
    let mut surface = Surface::from_handle(&interface, window_handle)?;
    let mut scene = Scene::build(&interface)?;
    let pipeline = Pipeline::create(&interface)?;

    event_loop
        .run(move |event, elwt| match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                interface.wait_for_gpu().unwrap();
                elwt.exit();
            }
            Event::AboutToWait => {
                unsafe {
                    interface.command_allocator.Reset().unwrap();
                    interface
                        .command_list
                        .Reset(&interface.command_allocator, None)
                        .unwrap();
                }

                scene.update(&interface);
                render(&interface, &mut scene, &pipeline, &surface).unwrap();
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                surface.resize(&interface).unwrap();
            }
            _ => (),
        })
        .unwrap();

    Ok(())
}
