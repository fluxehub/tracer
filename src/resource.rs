use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use lazy_static::lazy_static;
use windows::core::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

lazy_static! {
    pub static ref NO_AA: DXGI_SAMPLE_DESC = DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
    };
    pub static ref UPLOAD_HEAP: D3D12_HEAP_PROPERTIES = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_UPLOAD,
        ..Default::default()
    };
    pub static ref DEFAULT_HEAP: D3D12_HEAP_PROPERTIES = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        ..Default::default()
    };
    pub static ref BASIC_BUFFER_DESC: D3D12_RESOURCE_DESC = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: 0, // Will be changed in copies
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: *NO_AA,
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        ..Default::default()
    };
}

pub struct OpaqueResource(ID3D12Resource);

impl From<OpaqueResource> for ID3D12Resource {
    fn from(resource: OpaqueResource) -> Self {
        resource.0
    }
}

pub struct UploadResource<T> {
    type_: PhantomData<T>,
    resource: ID3D12Resource,
    size: usize,
}

pub struct ResourceBuffer<'a, T> {
    resource: &'a ID3D12Resource,
    data: &'a mut [T],
}

impl<T> Index<usize> for ResourceBuffer<'_, T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.data[index]
    }
}

impl<T> IndexMut<usize> for ResourceBuffer<'_, T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.data[index]
    }
}

impl<T: Copy> ResourceBuffer<'_, T> {
    pub fn copy_from_slice(&mut self, slice: &[T]) {
        assert_eq!(slice.len(), self.data.len());
        self.data.copy_from_slice(slice);
    }

    pub fn copy_from_slice_at(&mut self, slice: &[T], offset: usize) {
        assert!(offset + slice.len() <= self.data.len());
        self.data[offset..offset + slice.len()].copy_from_slice(slice);
    }
}

impl<T> Drop for ResourceBuffer<'_, T> {
    fn drop(&mut self) {
        unsafe {
            self.resource.Unmap(0, None);
        }
    }
}

impl OpaqueResource {
    pub fn get_gpu_virtual_address(&self) -> u64 {
        unsafe { self.0.GetGPUVirtualAddress() }
    }

    pub fn get_desc(&self) -> D3D12_RESOURCE_DESC {
        unsafe { self.0.GetDesc() }
    }
}

impl<T> UploadResource<T> {
    fn from_resource(resource: ID3D12Resource, size: usize) -> Self {
        Self {
            type_: PhantomData,
            resource,
            size,
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn get_buffer(&self) -> Result<ResourceBuffer<T>> {
        unsafe {
            let mut buffer_ptr = std::ptr::null_mut();
            self.resource.Map(0, None, Some(&mut buffer_ptr))?;
            let buffer = std::slice::from_raw_parts_mut(buffer_ptr as *mut T, self.size);
            Ok(ResourceBuffer {
                resource: &self.resource,
                data: buffer,
            })
        }
    }

    pub fn get_gpu_virtual_address(&self) -> u64 {
        unsafe { self.resource.GetGPUVirtualAddress() }
    }
}

impl<T> From<UploadResource<T>> for ID3D12Resource {
    fn from(resource: UploadResource<T>) -> Self {
        resource.resource
    }
}

pub struct ResourceFactory {
    device: ID3D12Device5,
}

impl ResourceFactory {
    pub fn new(device: ID3D12Device5) -> Self {
        Self { device }
    }

    fn create_d3d12_resource(
        &self,
        name: PCWSTR,
        heap_properties: D3D12_HEAP_PROPERTIES,
        buffer_flags: Option<D3D12_RESOURCE_FLAGS>,
        initial_state: Option<D3D12_RESOURCE_STATES>,
        size: u64,
    ) -> Result<ID3D12Resource> {
        let mut desc = *BASIC_BUFFER_DESC;
        desc.Width = size;
        if let Some(flags) = buffer_flags {
            desc.Flags = flags;
        }

        let mut resource = None;

        unsafe {
            self.device.CreateCommittedResource(
                &heap_properties,
                D3D12_HEAP_FLAG_NONE,
                &desc,
                initial_state.unwrap_or(D3D12_RESOURCE_STATE_COMMON),
                None,
                &mut resource,
            )?;
        }

        let resource: ID3D12Resource = resource.unwrap();
        unsafe { resource.SetName(name)? };

        Ok(resource)
    }

    pub fn create_upload_resource<T>(
        &self,
        name: PCWSTR,
        buffer_flags: Option<D3D12_RESOURCE_FLAGS>,
        initial_state: Option<D3D12_RESOURCE_STATES>,
        size: u64,
    ) -> Result<UploadResource<T>> {
        let resource = self.create_d3d12_resource(
            name,
            *UPLOAD_HEAP,
            buffer_flags,
            initial_state,
            size * std::mem::size_of::<T>() as u64,
        )?;

        Ok(UploadResource::from_resource(resource, size as usize))
    }

    pub fn create_upload_resource_from_slice<T: Copy>(
        &self,
        name: PCWSTR,
        buffer_flags: Option<D3D12_RESOURCE_FLAGS>,
        initial_state: Option<D3D12_RESOURCE_STATES>,
        data: &[T],
    ) -> Result<UploadResource<T>> {
        let resource =
            self.create_upload_resource(name, buffer_flags, initial_state, data.len() as u64)?;

        {
            let mut buffer = resource.get_buffer()?;
            buffer.copy_from_slice(data);
        }

        Ok(resource)
    }

    pub fn create_gpu_resource(
        &self,
        name: PCWSTR,
        buffer_flags: Option<D3D12_RESOURCE_FLAGS>,
        initial_state: Option<D3D12_RESOURCE_STATES>,
        size: u64,
    ) -> Result<OpaqueResource> {
        let resource =
            self.create_d3d12_resource(name, *DEFAULT_HEAP, buffer_flags, initial_state, size)?;

        Ok(OpaqueResource(resource))
    }
}

impl<T> From<UploadResource<T>> for OpaqueResource {
    fn from(resource: UploadResource<T>) -> Self {
        OpaqueResource(resource.resource)
    }
}
