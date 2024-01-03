use crate::device_interface::DeviceInterface;
use crate::imports::*;
use crate::resource::{OpaqueResource, ResourceBuffer, UploadResource};
use nalgebra::{Matrix4, Vector3};
use ouroboros::self_referencing;
use windows::Win32::System::SystemInformation::GetTickCount;

const QUAD_VTX: [f32; 18] = [
    -1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 1.0,
];

const CUBE_VTX: [f32; 24] = [
    -1.0, -1.0, -1.0, 1.0, -1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0, 1.0, 1.0, -1.0,
    1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];

const CUBE_IDX: [u16; 36] = [
    4, 6, 0, 2, 0, 6, 0, 1, 4, 5, 4, 1, 0, 2, 1, 3, 1, 2, 1, 3, 5, 7, 5, 3, 2, 6, 3, 7, 3, 6, 4, 5,
    6, 7, 6, 5,
];

const NUM_INSTANCES: u32 = 3;

#[self_referencing]
struct Instances {
    resource: UploadResource<D3D12_RAYTRACING_INSTANCE_DESC>,
    #[borrows(resource)]
    #[covariant]
    buffer: ResourceBuffer<'this, D3D12_RAYTRACING_INSTANCE_DESC>,
}

pub struct Scene {
    _quad_buffer: OpaqueResource,
    _cube_buffer: OpaqueResource,
    _cube_index_buffer: OpaqueResource,

    _quad_blas: OpaqueResource,
    _cube_blas: OpaqueResource,

    tlas: OpaqueResource,
    tlas_scratch: OpaqueResource,

    instances: Instances,
}

fn make_acceleration_structure(
    interface: &DeviceInterface,
    inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
) -> Result<(OpaqueResource, u64)> {
    let mut prebuild_info = Default::default();
    unsafe {
        interface
            .device
            .GetRaytracingAccelerationStructurePrebuildInfo(&inputs, &mut prebuild_info);
    }

    let update_scratch_size = prebuild_info.UpdateScratchDataSizeInBytes;

    let scratch = interface.resource_factory.create_gpu_resource(
        w!("Scratch Buffer"),
        Some(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
        None,
        prebuild_info.ScratchDataSizeInBytes,
    )?;

    let acceleration_structure = interface.resource_factory.create_gpu_resource(
        w!("Acceleration Structure"),
        Some(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
        Some(D3D12_RESOURCE_STATE_RAYTRACING_ACCELERATION_STRUCTURE),
        prebuild_info.ResultDataMaxSizeInBytes,
    )?;

    let build_desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
        DestAccelerationStructureData: acceleration_structure.get_gpu_virtual_address(),
        Inputs: inputs,
        ScratchAccelerationStructureData: scratch.get_gpu_virtual_address(),
        ..Default::default()
    };

    let command_list = &interface.command_list;
    unsafe {
        interface.command_allocator.Reset()?;
        command_list.Reset(&interface.command_allocator, None)?;
        command_list.BuildRaytracingAccelerationStructure(&build_desc, None);
        command_list.Close()?;
        let command_list = Some(command_list.can_clone_into());
        interface.queue.ExecuteCommandLists(&[command_list]);
    }

    interface.wait_for_gpu()?;

    Ok((acceleration_structure, update_scratch_size))
}

fn make_blas<V, I>(
    interface: &DeviceInterface,
    vertex_buffer: &UploadResource<V>,
    index_buffer: Option<&UploadResource<I>>,
) -> Result<OpaqueResource> {
    let geometry_desc = D3D12_RAYTRACING_GEOMETRY_DESC {
        Type: D3D12_RAYTRACING_GEOMETRY_TYPE_TRIANGLES,
        Flags: D3D12_RAYTRACING_GEOMETRY_FLAG_OPAQUE,
        Anonymous: D3D12_RAYTRACING_GEOMETRY_DESC_0 {
            Triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
                Transform3x4: 0,
                IndexFormat: if index_buffer.is_some() {
                    DXGI_FORMAT_R16_UINT
                } else {
                    DXGI_FORMAT_UNKNOWN
                },
                VertexFormat: DXGI_FORMAT_R32G32B32_FLOAT,
                IndexCount: index_buffer.map(|ib| ib.len() as u32).unwrap_or(0),
                VertexCount: (vertex_buffer.len() / 3) as u32,
                IndexBuffer: index_buffer
                    .map(|ib| ib.get_gpu_virtual_address())
                    .unwrap_or(0),
                VertexBuffer: D3D12_GPU_VIRTUAL_ADDRESS_AND_STRIDE {
                    StartAddress: vertex_buffer.get_gpu_virtual_address(),
                    StrideInBytes: std::mem::size_of::<f32>() as u64 * 3,
                },
            },
        },
    };

    let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
        Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_BOTTOM_LEVEL,
        Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PREFER_FAST_TRACE,
        NumDescs: 1,
        DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
        Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
            pGeometryDescs: &geometry_desc,
        },
        ..Default::default()
    };

    let (blas, _) = make_acceleration_structure(interface, inputs)?;
    Ok(blas)
}

fn make_tlas(
    interface: &DeviceInterface,
    instances: &UploadResource<D3D12_RAYTRACING_INSTANCE_DESC>,
) -> Result<(OpaqueResource, u64)> {
    let inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
        Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
        Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_ALLOW_UPDATE, // TODO: Add flag for fast trace
        NumDescs: instances.len() as u32,
        DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
        Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
            InstanceDescs: instances.get_gpu_virtual_address(),
        },
    };

    make_acceleration_structure(interface, inputs)
}

fn update_transforms(instances: &mut ResourceBuffer<D3D12_RAYTRACING_INSTANCE_DESC>) {
    let mut set = |i: usize, m: Matrix4<f32>| {
        instances[i]
            .Transform
            .copy_from_slice(&m.transpose().as_slice()[..12]);
    };

    let time = unsafe { GetTickCount() as f32 / 1000.0 };
    let mut cube = Matrix4::from_euler_angles(time / 2.0, time / 3.0, time / 5.0);
    cube = cube.append_translation(&Vector3::new(-1.5, 2.0, 2.0));
    set(0, cube);

    let mut mirror = Matrix4::from_scaled_axis(
        (&Vector3::x() * -1.8) + (&Vector3::y() * (time.sin() / 8.0 + 1.0)),
    );
    mirror = mirror.append_translation(&Vector3::new(2.0, 2.0, 2.0));
    set(1, mirror);

    let mut floor = Matrix4::new_scaling(5.0);
    floor = floor.append_translation(&Vector3::new(0.0, 0.0, 2.0));
    set(2, floor);
}

impl Scene {
    pub fn build(interface: &DeviceInterface) -> Result<Self> {
        let quad_buffer = interface
            .resource_factory
            .create_upload_resource_from_slice(w!("Quad Buffer"), None, None, &QUAD_VTX)?;

        let cube_buffer = interface
            .resource_factory
            .create_upload_resource_from_slice(w!("Cube Buffer"), None, None, &CUBE_VTX)?;

        let cube_index_buffer = interface
            .resource_factory
            .create_upload_resource_from_slice(w!("Cube Index Buffer"), None, None, &CUBE_IDX)?;

        // TODO: Name these resources
        let quad_blas = make_blas::<f32, ()>(interface, &quad_buffer, None)?;
        let cube_blas = make_blas(interface, &cube_buffer, Some(&cube_index_buffer))?;

        let instances = interface.resource_factory.create_upload_resource(
            w!("Instances"),
            None,
            None,
            NUM_INSTANCES as u64,
        )?;

        {
            let mut instances_buffer = instances.get_buffer()?;

            for i in 0..NUM_INSTANCES {
                instances_buffer[i as usize] = D3D12_RAYTRACING_INSTANCE_DESC {
                    _bitfield1: i | (1 << 24),
                    AccelerationStructure: if i == 0 {
                        cube_blas.get_gpu_virtual_address()
                    } else {
                        quad_blas.get_gpu_virtual_address()
                    },
                    ..Default::default()
                }
            }

            update_transforms(&mut instances_buffer);
        }

        let (tlas, scratch_size) = make_tlas(interface, &instances)?;
        let tlas_scratch = interface.resource_factory.create_gpu_resource(
            w!("TLAS Scratch"),
            Some(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
            None,
            scratch_size,
        )?;

        let instances = InstancesBuilder {
            resource: instances,
            buffer_builder: |resource| resource.get_buffer().unwrap(),
        }
        .build();

        Ok(Scene {
            _quad_buffer: quad_buffer.into(),
            _cube_buffer: cube_buffer.into(),
            _cube_index_buffer: cube_index_buffer.into(),
            _quad_blas: quad_blas,
            _cube_blas: cube_blas,
            tlas,
            tlas_scratch,
            instances,
        })
    }

    pub fn update(&mut self, interface: &DeviceInterface) {
        self.instances.with_buffer_mut(|instances| {
            update_transforms(instances);
        });

        let desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
            DestAccelerationStructureData: self.tlas.get_gpu_virtual_address(),
            Inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
                Type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE_TOP_LEVEL,
                Flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAG_PERFORM_UPDATE,
                NumDescs: NUM_INSTANCES,
                DescsLayout: D3D12_ELEMENTS_LAYOUT_ARRAY,
                Anonymous: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS_0 {
                    InstanceDescs: self.instances.borrow_resource().get_gpu_virtual_address(),
                },
            },
            SourceAccelerationStructureData: self.tlas.get_gpu_virtual_address(),
            ScratchAccelerationStructureData: self.tlas_scratch.get_gpu_virtual_address(),
            ..Default::default()
        };

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                UAV: std::mem::ManuallyDrop::new(D3D12_RESOURCE_UAV_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(&self.tlas) },
                }),
            },
            ..Default::default()
        };

        unsafe {
            interface
                .command_list
                .BuildRaytracingAccelerationStructure(&desc, None);
            interface.command_list.ResourceBarrier(&[barrier]);
        }
    }

    pub fn bind(&self, interface: &DeviceInterface) {
        unsafe {
            interface
                .command_list
                .SetComputeRootShaderResourceView(1, self.tlas.get_gpu_virtual_address())
        }
    }
}
