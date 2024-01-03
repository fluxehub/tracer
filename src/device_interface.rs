use crate::resource::ResourceFactory;
use windows::{
    core::*,
    Win32::Graphics::{Direct3D::*, Direct3D12::*},
};

pub struct DeviceInterface {
    pub device: ID3D12Device5,
    pub queue: ID3D12CommandQueue,
    pub fence: ID3D12Fence,
    pub command_allocator: ID3D12CommandAllocator,
    pub command_list: ID3D12GraphicsCommandList4,
    pub resource_factory: ResourceFactory,
}

impl DeviceInterface {
    pub fn create() -> Result<Self> {
        #[cfg(debug_assertions)]
        {
            let mut debug = None;
            unsafe {
                D3D12GetDebugInterface(&mut debug)?;
                let debug: ID3D12Debug1 = debug.unwrap();
                debug.EnableDebugLayer();
                debug.SetEnableGPUBasedValidation(true);
            }
        }

        let mut device = None;
        unsafe {
            D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_1, &mut device)?;
        }

        let device: ID3D12Device5 = device.unwrap();
        let resource_factory = ResourceFactory::new(device.clone()); // TODO: Can we replace with a reference?

        #[cfg(debug_assertions)]
        {
            let info_queue: ID3D12InfoQueue = device.cast()?;
            unsafe {
                info_queue.SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_CORRUPTION, true)?;
                info_queue.SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_ERROR, true)?;
                info_queue.SetBreakOnSeverity(D3D12_MESSAGE_SEVERITY_WARNING, true)?;
            }
        }

        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        };

        let queue = unsafe { device.CreateCommandQueue(&queue_desc)? };
        let fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? };

        let command_allocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)? };

        let command_list = unsafe {
            device.CreateCommandList1(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                D3D12_COMMAND_LIST_FLAG_NONE,
            )?
        };

        Ok(Self {
            device,
            queue,
            fence,
            command_allocator,
            command_list,
            resource_factory,
        })
    }

    pub fn wait_for_gpu(&self) -> Result<()> {
        unsafe {
            let fence = self.fence.GetCompletedValue() + 1;
            self.queue.Signal(&self.fence, fence)?;
            self.fence.SetEventOnCompletion(fence, None)
        }
    }
}
