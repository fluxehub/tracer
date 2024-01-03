use crate::device_interface::DeviceInterface;
use crate::imports::*;
use crate::resource::NO_AA;
use std::cmp::max;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

pub struct Surface {
    pub target: ID3D12Resource,
    window: HWND,
    swap_chain: IDXGISwapChain4,
    uav_heap: ID3D12DescriptorHeap,
}

fn barrier(
    command_list: &ID3D12GraphicsCommandList4,
    resource: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    let barrier = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            // TODO: Is this a memory leak?
            Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: unsafe { std::mem::transmute_copy(resource) },
                StateBefore: before,
                StateAfter: after,
                ..Default::default()
            }),
        },
        ..Default::default()
    };

    unsafe { command_list.ResourceBarrier(&[barrier]) };
}

fn internal_resize(
    interface: &DeviceInterface,
    window: HWND,
    swap_chain: &IDXGISwapChain4,
    uav_heap: &ID3D12DescriptorHeap,
) -> Result<ID3D12Resource> {
    let mut rect = Default::default();
    unsafe { GetClientRect(window, &mut rect)? };
    let width = max(rect.right - rect.left, 1) as u32;
    let height = max(rect.bottom - rect.top, 1) as u32;

    interface.wait_for_gpu()?; // Make sure the device is idle before we resize

    unsafe { swap_chain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, 0)? };

    let rt_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: width as u64,
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: *NO_AA,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
        ..Default::default()
    };

    let mut render_target = None;

    let default_heap = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        ..Default::default()
    };

    // Using ID3D12Resource instead of the wrapper until I decide whether I want to use the wrapper
    // since the render target's use of the resource is more complex than all the other resources
    // TODO: Think about this!
    unsafe {
        interface.device.CreateCommittedResource(
            &default_heap,
            D3D12_HEAP_FLAG_NONE,
            &rt_desc,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            None,
            &mut render_target,
        )?
    };

    let render_target: ID3D12Resource = render_target.unwrap();

    let uav_desc = D3D12_UNORDERED_ACCESS_VIEW_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
        ..Default::default()
    };

    unsafe {
        interface.device.CreateUnorderedAccessView(
            &render_target,
            None,
            Some(&uav_desc),
            uav_heap.GetCPUDescriptorHandleForHeapStart(),
        )
    };

    Ok(render_target)
}

impl Surface {
    pub fn from_handle(interface: &DeviceInterface, window: HWND) -> Result<Self> {
        let factory: IDXGIFactory2 = if cfg!(debug_assertions) {
            unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG)? }
        } else {
            unsafe { CreateDXGIFactory2(0)? }
        };

        let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: *NO_AA,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };

        let swap_chain = unsafe {
            factory
                .CreateSwapChainForHwnd(&interface.queue, window, &swap_chain_desc, None, None)?
                .cast()?
        };

        let uav_heap_desc = D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: 1,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            ..Default::default()
        };

        let uav_heap = unsafe { interface.device.CreateDescriptorHeap(&uav_heap_desc)? };
        let target = internal_resize(interface, window, &swap_chain, &uav_heap)?;

        Ok(Self {
            target,
            window,
            swap_chain,
            uav_heap,
        })
    }

    pub fn resize(&mut self, interface: &DeviceInterface) -> Result<()> {
        self.target = internal_resize(interface, self.window, &self.swap_chain, &self.uav_heap)?;
        Ok(())
    }

    pub fn bind(&self, interface: &DeviceInterface) -> Result<D3D12_RESOURCE_DESC> {
        let command_list = &interface.command_list;
        unsafe {
            command_list.SetDescriptorHeaps(&[Some(self.uav_heap.clone())]);
            let uav_table = self.uav_heap.GetGPUDescriptorHandleForHeapStart();
            command_list.SetComputeRootDescriptorTable(0, uav_table);
            Ok(self.target.GetDesc())
        }
    }

    pub fn present(&self, interface: &DeviceInterface) -> Result<()> {
        let command_list = &interface.command_list;
        let back_buffer: ID3D12Resource = unsafe {
            self.swap_chain
                .GetBuffer(self.swap_chain.GetCurrentBackBufferIndex())?
        };

        unsafe { back_buffer.SetName(w!("Back Buffer"))? };

        barrier(
            command_list,
            &self.target,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );

        barrier(
            command_list,
            &back_buffer,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );

        unsafe {
            command_list.CopyResource(&back_buffer, &self.target);
        }

        barrier(
            command_list,
            &back_buffer,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_PRESENT,
        );

        barrier(
            command_list,
            &self.target,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );

        unsafe {
            command_list.Close()?;
            let command_list = Some(command_list.can_clone_into());
            interface.queue.ExecuteCommandLists(&[command_list]);
        }

        interface.wait_for_gpu()?;
        unsafe { self.swap_chain.Present(1, 0).ok() }
    }
}
