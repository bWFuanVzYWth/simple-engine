//! 构建期核对绑定、资源类型与 float4 布局；不替代运行时任意外部资源的 wgpu 验证。

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("读取反射 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("解析反射 {path} 失败：{source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} 的 shader ABI 不匹配：{detail}")]
    Layout { path: PathBuf, detail: String },
}

pub fn verify(output: &Path) -> Result<(), Error> {
    println!("cargo::rerun-if-changed=build_abi.rs");
    for target in ["spv", "wgsl"] {
        for (stage, entry) in [("vert", "vertex_main"), ("frag", "fragment_main")] {
            let path = output.join(format!("render2d.{stage}.{target}.json"));
            let bytes = fs::read(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            let reflection: Value =
                serde_json::from_slice(&bytes).map_err(|source| Error::Json {
                    path: path.clone(),
                    source,
                })?;
            verify_reflection(&reflection, entry)
                .map_err(|detail| Error::Layout { path, detail })?;
        }
    }
    let path = output.join("render2d.abi.rs");
    fs::write(
        &path,
        "/// 单个 Instance 的 shader 字节跨度。\npub const INSTANCE_STRIDE: u64 = 64;\n\
         /// View uniform 的 shader 字节大小。\npub const VIEW_SIZE: u64 = 32;\n",
    )
    .map_err(|source| Error::Io { path, source })?;
    Ok(())
}

fn verify_reflection(reflection: &Value, entry: &str) -> Result<(), String> {
    let entries = reflection["entryPoints"]
        .as_array()
        .ok_or("缺少 entryPoints")?;
    if !entries.iter().any(|value| value["name"] == entry) {
        return Err(format!("缺少入口 {entry}"));
    }
    let parameters = reflection["parameters"]
        .as_array()
        .ok_or("缺少 parameters")?;
    if parameters.len() != 4 {
        return Err("资源数量不匹配".into());
    }
    let instances = parameter(parameters, "instances", 0, 0)?;
    let view = parameter(parameters, "view", 0, 1)?;
    check_resource(&instances["type"], "resource", Some("structuredBuffer"))?;
    check_resource(&view["type"], "constantBuffer", None)?;
    check_struct(
        &instances["type"]["resultType"],
        "Instance",
        &["rect", "uv", "color", "params"],
    )?;
    check_struct(
        &view["type"]["elementType"],
        "View",
        &["transform", "pixel"],
    )?;
    let view_size = view["type"]["elementVarLayout"]["binding"]["size"].as_u64();
    if view_size != Some(32) {
        return Err(format!("View size = {view_size:?}，期望 32"));
    }
    let image = parameter(parameters, "image", 1, 0)?;
    check_resource(&image["type"], "resource", Some("texture2D"))?;
    let texel = &image["type"]["resultType"];
    if texel["kind"] != "vector"
        || texel["elementCount"] != 4
        || texel["elementType"]["scalarType"] != "float32"
    {
        return Err("image texel 必须为 float4".into());
    }
    let sampler = parameter(parameters, "image_sampler", 1, 1)?;
    check_resource(&sampler["type"], "samplerState", None)?;
    Ok(())
}

fn check_resource(ty: &Value, kind: &str, shape: Option<&str>) -> Result<(), String> {
    if ty["kind"] != kind
        || shape.is_some_and(|shape| ty["baseShape"] != shape)
        || ty["access"].as_str().is_some_and(|access| access != "read")
    {
        return Err(format!("资源类型不匹配：{ty}"));
    }
    Ok(())
}

fn parameter<'a>(
    parameters: &'a [Value],
    name: &str,
    group: u64,
    index: u64,
) -> Result<&'a Value, String> {
    let parameter = parameters
        .iter()
        .find(|value| value["name"] == name)
        .ok_or_else(|| format!("缺少资源 {name}"))?;
    let binding = &parameter["binding"];
    if binding["kind"] != "descriptorTableSlot"
        || binding["space"].as_u64().unwrap_or(0) != group
        || binding["index"].as_u64() != Some(index)
    {
        return Err(format!(
            "{name} binding = {binding}，期望 group {group} binding {index}"
        ));
    }
    Ok(parameter)
}

fn check_struct(ty: &Value, name: &str, expected: &[&str]) -> Result<(), String> {
    if ty["kind"] != "struct" || ty["name"] != name {
        return Err(format!("缺少结构 {name}"));
    }
    let fields = ty["fields"]
        .as_array()
        .ok_or_else(|| format!("{name} 缺少 fields"))?;
    if fields.len() != expected.len() {
        return Err(format!("{name} 字段数量不匹配"));
    }
    for (i, (field, expected)) in fields.iter().zip(expected).enumerate() {
        let binding = &field["binding"];
        let ty = &field["type"];
        if field["name"] != *expected
            || ty["kind"] != "vector"
            || ty["elementCount"] != 4
            || ty["elementType"]["scalarType"] != "float32"
            || binding["offset"].as_u64() != Some((i * 16) as u64)
            || binding["size"].as_u64() != Some(16)
        {
            return Err(format!(
                "{name}.{expected} 不是位于 {} 的 float4：{field}",
                i * 16
            ));
        }
    }
    Ok(())
}
