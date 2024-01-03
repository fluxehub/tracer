use crate::device_interface::DeviceInterface;
use crate::imports::*;
use crate::resource::{OpaqueResource, UploadResource};
use std::ffi::c_void;

const SHADER_BYTES: &[u8] = include_bytes!("shaders/shaders.bin");

const NUM_SHADER_IDS: u32 = 3;

fn create_root_signature(interface: &DeviceInterface) -> Result<ID3D12RootSignature> {
    let uav_range = D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
        NumDescriptors: 1,
        ..Default::default()
    };

    let params = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                    NumDescriptorRanges: 1,
                    pDescriptorRanges: &uav_range,
                },
            },
            ..Default::default()
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                Descriptor: D3D12_ROOT_DESCRIPTOR {
                    ShaderRegister: 0,
                    RegisterSpace: 0,
                },
            },
            ..Default::default()
        },
    ];

    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: params.len() as u32,
        pParameters: params.as_ptr(),
        ..Default::default()
    };

    let mut blob = None;
    let mut error = None;
    unsafe {
        D3D12SerializeRootSignature(
            &desc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            Some(&mut error),
        )?
    };

    if let Some(error) = error {
        let error = unsafe { std::ffi::CStr::from_ptr(error.GetBufferPointer().cast()) };
        panic!(
            "Error serializing root signature: {}",
            error.to_string_lossy()
        );
    }

    let blob = blob.unwrap();

    unsafe {
        interface.device.CreateRootSignature(
            0,
            std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()),
        )
    }
}

pub struct Pipeline {
    root_signature: ID3D12RootSignature,
    pso: ID3D12StateObject,
    shader_ids: OpaqueResource,
}

impl Pipeline {
    pub fn create(interface: &DeviceInterface) -> Result<Self> {
        let root_signature = create_root_signature(interface)?;

        let lib = D3D12_DXIL_LIBRARY_DESC {
            DXILLibrary: D3D12_SHADER_BYTECODE {
                pShaderBytecode: SHADER_BYTES.as_ptr().cast(),
                BytecodeLength: SHADER_BYTES.len(),
            },
            ..Default::default()
        };

        let hit_group = D3D12_HIT_GROUP_DESC {
            HitGroupExport: w!("HitGroup"),
            Type: D3D12_HIT_GROUP_TYPE_TRIANGLES,
            ClosestHitShaderImport: w!("ClosestHit"),
            ..Default::default()
        };

        let shader_config = D3D12_RAYTRACING_SHADER_CONFIG {
            MaxPayloadSizeInBytes: 20,
            MaxAttributeSizeInBytes: 8,
        };

        let global_signature = D3D12_GLOBAL_ROOT_SIGNATURE {
            pGlobalRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
        };

        let pipeline_cfg = D3D12_RAYTRACING_PIPELINE_CONFIG {
            MaxTraceRecursionDepth: 10,
        };

        let sub_objects = [
            D3D12_STATE_SUBOBJECT {
                Type: D3D12_STATE_SUBOBJECT_TYPE_DXIL_LIBRARY,
                pDesc: &lib as *const _ as *const c_void,
            },
            D3D12_STATE_SUBOBJECT {
                Type: D3D12_STATE_SUBOBJECT_TYPE_HIT_GROUP,
                pDesc: &hit_group as *const _ as *const c_void,
            },
            D3D12_STATE_SUBOBJECT {
                Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_SHADER_CONFIG,
                pDesc: &shader_config as *const _ as *const c_void,
            },
            D3D12_STATE_SUBOBJECT {
                Type: D3D12_STATE_SUBOBJECT_TYPE_GLOBAL_ROOT_SIGNATURE,
                pDesc: &global_signature as *const _ as *const c_void,
            },
            D3D12_STATE_SUBOBJECT {
                Type: D3D12_STATE_SUBOBJECT_TYPE_RAYTRACING_PIPELINE_CONFIG,
                pDesc: &pipeline_cfg as *const _ as *const c_void,
            },
        ];

        let desc = D3D12_STATE_OBJECT_DESC {
            Type: D3D12_STATE_OBJECT_TYPE_RAYTRACING_PIPELINE,
            NumSubobjects: sub_objects.len() as u32,
            pSubobjects: sub_objects.as_ptr(),
        };

        let pso: ID3D12StateObject = unsafe { interface.device.CreateStateObject(&desc)? };

        let shader_ids: UploadResource<u8> = interface.resource_factory.create_upload_resource(
            w!("Shader IDs"),
            None,
            None,
            NUM_SHADER_IDS as u64 * D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as u64,
        )?;

        let props: ID3D12StateObjectProperties = pso.cast()?;

        {
            let mut data = shader_ids.get_buffer()?;
            let names = [w!("RayGeneration"), w!("Miss"), w!("HitGroup")];
            for (i, name) in names.into_iter().enumerate() {
                let id = unsafe { props.GetShaderIdentifier(name) };
                let id_slice: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        id.cast(),
                        D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as usize,
                    )
                };
                data.copy_from_slice_at(
                    id_slice,
                    i * D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as usize,
                )
            }
        }

        Ok(Self {
            root_signature,
            pso,
            shader_ids: shader_ids.into(),
        })
    }

    pub fn bind(&self, interface: &DeviceInterface) {
        let command_list = &interface.command_list;
        unsafe {
            command_list.SetPipelineState1(&self.pso);
            command_list.SetComputeRootSignature(&self.root_signature);
        }
    }

    pub fn create_rays_description(
        &self,
        surface_desc: &D3D12_RESOURCE_DESC,
    ) -> D3D12_DISPATCH_RAYS_DESC {
        D3D12_DISPATCH_RAYS_DESC {
            RayGenerationShaderRecord: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                StartAddress: self.shader_ids.get_gpu_virtual_address(),
                SizeInBytes: D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64,
            },
            MissShaderTable: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                StartAddress: self.shader_ids.get_gpu_virtual_address()
                    + D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as u64,
                SizeInBytes: D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64,
                ..Default::default()
            },
            HitGroupTable: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                StartAddress: self.shader_ids.get_gpu_virtual_address()
                    + 2 * D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT as u64,
                SizeInBytes: D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES as u64,
                ..Default::default()
            },
            Width: surface_desc.Width as u32,
            Height: surface_desc.Height,
            Depth: 1,
            ..Default::default()
        }
    }
}
