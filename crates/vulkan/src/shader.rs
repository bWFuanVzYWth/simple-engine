use crate::{Device, Error, vk};
use std::io::Cursor;

pub(crate) struct ShaderModule<'a> {
    pub(crate) raw: vk::ShaderModule,
    device: &'a Device,
}

impl<'a> ShaderModule<'a> {
    pub(crate) fn new(device: &'a Device, bytes: &[u8]) -> Result<Self, Error> {
        device.ensure_active()?;
        let code = decode_spirv(bytes)?;
        let raw = unsafe {
            device
                .raw
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)?
        };
        Ok(Self { raw, device })
    }
}

impl Drop for ShaderModule<'_> {
    fn drop(&mut self) {
        unsafe { self.device.raw.destroy_shader_module(self.raw, None) };
    }
}

pub(crate) fn decode_spirv(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    Ok(ash::util::read_spv(&mut Cursor::new(bytes))?)
}
