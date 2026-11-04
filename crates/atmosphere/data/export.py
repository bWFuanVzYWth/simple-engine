"""从只读 Sky Tracer 9e9430d 导出物理系数和拟合坐标；不运行 GPU 或烘焙辐射场。"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--reference", type=Path, help="另运行原 WGSL GPU 求解，冻结独立测试 oracle")
    args = parser.parse_args()
    if sys.byteorder != "little":
        raise ValueError("export requires a little-endian host")
    root = Path(__file__).resolve().parents[3]
    source = args.source.resolve()
    commit = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    expected = "9e9430d52c50aa3df97fdf0fb6515abc67f44bef"
    if commit != expected:
        raise ValueError(f"source commit {commit} != {expected}")
    if subprocess.check_output(["git", "-C", str(source), "status", "--porcelain", "--", "crates/sky-realtime"], text=True).strip():
        raise ValueError("source sky-realtime has uncommitted changes")
    target = root / "temp/atmosphere/20261027/source-export"
    target.mkdir(parents=True, exist_ok=True)
    upstream = source / "crates/sky-realtime"
    files = subprocess.check_output(
        ["git", "-C", str(upstream), "ls-files", "-z", "--", "src", "data", "configs"]
    ).decode("utf-8").split("\0")
    for name in filter(None, files):
        destination = target / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(upstream / name, destination)
    manifest = '''[package]
name = "sky-atmosphere-export"
version = "0.0.0"
edition = "2024"
license = "GPL-3.0-only"
autolib = false
autobins = false
[workspace]
[[bin]]
name = "sky-atmosphere-export"
path = "src/lib.rs"
[dependencies]
bytemuck = { version = "1.25", features = ["derive"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = { version = "1.0", features = ["float_roundtrip"] }
wgpu = "29.0.3"
pollster = "0.4"
'''
    (target / "Cargo.toml").write_text(manifest, encoding="utf-8")
    lib = target / "src/lib.rs"
    text = lib.read_text(encoding="utf-8").replace(
        "#[derive(Clone, Copy, Pod, Zeroable)]\nstruct Params",
        "#[derive(Clone, Copy, Pod, Zeroable, Serialize)]\nstruct Params",
    )
    if "Zeroable, Serialize)]\nstruct Params" not in text:
        raise ValueError("upstream Params declaration changed")
    text += '''
include!("reference.rs");
fn main() -> Result<()> {
    let output = std::path::PathBuf::from(std::env::args_os().nth(1).ok_or("output path")?);
    let config = Config::balanced();
    let medium = Medium::new(&Model::earth()?, &Wavelengths::optimized_four(), &config, 0.18)?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("medium.bin"), bytemuck::cast_slice(&medium.data))?;
    std::fs::write(output.join("params.bin"), bytemuck::bytes_of(&medium.params))?;
    std::fs::write(output.join("params.json"), serde_json::to_vec_pretty(&medium.params)?)?;
    println!("{} float4 rows; {} B params; field={} B; resident={} B", medium.data.len(),
        std::mem::size_of::<Params>(), medium.field_bytes(&config), medium.resident_bytes(&config));
    if let Some(path)=std::env::args_os().nth(2) {pollster::block_on(reference(std::path::Path::new(&path)))?;}
    Ok(())
}
'''
    lib.write_text(text, encoding="utf-8")
    shutil.copyfile(Path(__file__).with_name("reference.rs"),target / "src/reference.rs")
    output = Path(__file__).resolve().parent
    subprocess.run(
        ["cargo", "run", "--release", "--manifest-path", str(target / "Cargo.toml"),
         "--target-dir", str(root / "target/atmosphere-export"), "--", str(output)]
        + ([str(args.reference.resolve())] if args.reference else []),
        cwd=root, check=True,
    )
    shutil.copyfile(upstream / "configs/mapping_fit.json", output / "mapping_fit.json")
    shutil.copyfile(upstream / "configs/wavelengths.json", output / "wavelengths.json")
    sums = {name: hashlib.sha256((output / name).read_bytes()).hexdigest()
            for name in ["medium.bin", "params.bin", "params.json", "mapping_fit.json", "wavelengths.json"]}
    (output / "sha256.json").write_text(json.dumps(sums, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(sums, indent=2))


if __name__ == "__main__":
    main()
