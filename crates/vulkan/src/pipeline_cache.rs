//! 驱动管线缓存只在设备初始化、阶段发布和销毁边界交换快照。

use crate::{Device, vk};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

const LIMIT: usize = 64 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"SEPC0001";
const PREFIX: usize = MAGIC.len() + 32;

pub(crate) struct Disk {
    path: PathBuf,
    dirty: AtomicBool,
}

impl Disk {
    pub(crate) fn new(directory: &Path, name: &str, props: &vk::PhysicalDeviceProperties) -> Self {
        let uuid: String = props
            .pipeline_cache_uuid
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let path = directory.join(format!(
            "{name}-{:08x}-{:08x}-{:08x}-{uuid}.bin",
            props.vendor_id, props.device_id, props.driver_version
        ));
        Self {
            path,
            dirty: AtomicBool::new(false),
        }
    }

    pub(crate) fn read(&self) -> Vec<u8> {
        let result: io::Result<Vec<u8>> = (|| {
            let file = File::open(&self.path)?;
            let size = file.metadata()?.len();
            if !(PREFIX as u64..=(LIMIT + PREFIX) as u64).contains(&size) {
                return Ok(Vec::new());
            }
            let mut file = file.take((LIMIT + PREFIX + 1) as u64);
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            if bytes.len() as u64 != size || !bytes.starts_with(MAGIC) {
                return Ok(Vec::new());
            }
            if blake3::hash(&bytes[PREFIX..]).as_bytes() != &bytes[MAGIC.len()..PREFIX] {
                return Ok(Vec::new());
            }
            bytes.drain(..PREFIX);
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => bytes,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    log::warn!("读取管线缓存失败 {}：{error}", self.path.display());
                }
                Vec::new()
            }
        }
    }

    pub(crate) fn used(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub(crate) fn flush(&mut self, snapshot: impl FnOnce() -> Option<Vec<u8>>) {
        if !*self.dirty.get_mut() {
            return;
        }
        if let Some(bytes) = snapshot().filter(|data| !data.is_empty() && data.len() <= LIMIT)
            && let Err(error) = self.write(&bytes)
        {
            log::warn!("保存管线缓存失败 {}：{error}", self.path.display());
            return;
        }
        *self.dirty.get_mut() = false;
    }

    fn write(&self, data: &[u8]) -> io::Result<()> {
        let parent = self.path.parent().expect("缓存路径由目录和文件名组成");
        fs::create_dir_all(parent)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = self.path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let written = (|| {
            file.write_all(MAGIC)?;
            file.write_all(blake3::hash(data).as_bytes())?;
            file.write_all(data)?;
            file.sync_all()
        })();
        drop(file);
        let result = written.and_then(|()| fs::rename(&temporary, &self.path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

pub(crate) struct Native {
    raw: vk::PipelineCache,
    disk: Disk,
}

impl Native {
    pub(crate) fn new(device: &Device, directory: &Path) -> Result<Self, vk::Result> {
        let props = device.physical.properties();
        let disk = Disk::new(directory, "native", &props);
        let bytes = disk.read();
        let initial = if compatible(&bytes, &props) {
            bytes.as_slice()
        } else {
            &[]
        };
        // SAFETY: 初始数据来自本应用的驱动快照，经完整性和设备身份校验。同步由 Vulkan 内部维护。
        let create = |data| unsafe {
            device.raw.create_pipeline_cache(
                &vk::PipelineCacheCreateInfo::default().initial_data(data),
                None,
            )
        };
        let raw = match create(initial) {
            Err(_) if !initial.is_empty() => create(&[])?,
            other => other?,
        };
        Ok(Self { raw, disk })
    }

    pub(crate) fn used(&self) -> vk::PipelineCache {
        self.disk.used();
        self.raw
    }

    pub(crate) fn flush(&mut self, device: &Device) {
        let raw = self.raw;
        self.disk.flush(|| match snapshot(device, raw) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("获取 Vulkan 管线缓存失败：{error}");
                None
            }
        });
    }

    pub(crate) fn destroy(mut self, device: &Device) {
        self.flush(device);
        // SAFETY: Context 已独占关闭；缓存与已创建的 pipeline 生命周期独立。
        unsafe { device.raw.destroy_pipeline_cache(self.raw, None) };
    }
}

fn snapshot(device: &Device, cache: vk::PipelineCache) -> Result<Option<Vec<u8>>, crate::Error> {
    let get = device.raw.fp_v1_0().get_pipeline_cache_data;
    let mut size = 0;
    // SAFETY: 缓存归属同一有效设备。调用者独占 Context，空数据指针仅查询长度。
    unsafe { get(device.raw.handle(), cache, &mut size, std::ptr::null_mut()) }.result()?;
    if size == 0 || size > LIMIT {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size)?;
    bytes.resize(size, 0);
    // SAFETY: 数据指针可写长度为 size。缓存和设备有效期同上。
    unsafe {
        get(
            device.raw.handle(),
            cache,
            &mut size,
            bytes.as_mut_ptr().cast(),
        )
    }
    .result()?;
    bytes.truncate(size);
    Ok(Some(bytes))
}

fn compatible(data: &[u8], props: &vk::PhysicalDeviceProperties) -> bool {
    // Vulkan v1 头固定 32B 且整数为小端。见 VkPipelineCacheHeaderVersionOne。
    data.len() >= 32
        && data[..4] == 32u32.to_le_bytes()
        && data[4..8] == 1u32.to_le_bytes()
        && data[8..12] == props.vendor_id.to_le_bytes()
        && data[12..16] == props.device_id.to_le_bytes()
        && data[16..32] == props.pipeline_cache_uuid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_and_foreign_vulkan_header() {
        let props = vk::PhysicalDeviceProperties::default()
            .vendor_id(10)
            .device_id(20)
            .pipeline_cache_uuid([4; 16]);
        let mut data: Vec<_> = [32u32, 1, 10, 20]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        data.extend([4; 16]);
        assert!(compatible(&data, &props));
        assert!(!compatible(&data[..31], &props));
        assert!(!compatible(&data, &props.pipeline_cache_uuid([8; 16])));
    }

    #[test]
    fn replaces_complete_snapshot_and_rejects_modified_payload() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("simple-engine-cache-test-{}", std::process::id()));
        let disk = Disk::new(&dir, "round-trip", &vk::PhysicalDeviceProperties::default());
        disk.write(b"first")?;
        disk.write(b"replacement")?;
        assert_eq!(disk.read(), b"replacement");
        let mut raw = fs::read(&disk.path)?;
        raw[PREFIX] ^= 1;
        fs::write(&disk.path, raw)?;
        assert!(disk.read().is_empty());
        fs::remove_file(disk.path)?;
        fs::remove_dir(dir)
    }

    #[test]
    #[ignore = "真实 Vulkan 设备的 native 和 wgpu 缓存快照"]
    fn device_pipeline_cache_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::temp_dir().join(format!("simple-engine-cache-gpu-{}", std::process::id()));
        let mut context = pollster::block_on(crate::interop::WgpuContext::compute())?;
        context.enable_pipeline_caches(&directory);
        let device = &context.native_ref().device;
        let props = device.physical.properties();
        let cache = Native::new(device, &directory)?;
        assert_ne!(cache.used(), vk::PipelineCache::null());
        cache.destroy(device);
        let native = Disk::new(&directory, "native", &props);
        assert!(compatible(&native.read(), &props));
        let cache = Native::new(device, &directory)?;
        assert_ne!(cache.used(), vk::PipelineCache::null());
        cache.destroy(device);
        assert!(context.pipeline_cache().is_some());
        context.flush_pipeline_caches();
        drop(context);
        let wgpu = Disk::new(&directory, "wgpu", &props);
        assert!(!wgpu.read().is_empty());
        let mut context = pollster::block_on(crate::interop::WgpuContext::compute())?;
        context.enable_pipeline_caches(&directory);
        assert!(
            context
                .pipeline_cache()
                .and_then(wgpu::PipelineCache::get_data)
                .is_some()
        );
        drop(context);
        fs::remove_file(native.path)?;
        fs::remove_file(wgpu.path)?;
        fs::remove_dir(directory)?;
        Ok(())
    }
}
