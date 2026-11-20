// SPDX-License-Identifier: GPL-3.0-only
/// 制核各阶段共用的标量与二维区间，对应 Rust 的 OpticalGrid。
module pupil_grid;
public enum PupilKind : uint {
    radial,
    mask_composite,
    mask_native
}
;
public struct OpticalGrid {
    PupilKind kind;
    uint side;
    uint node_count;
    uint dust_count;
    uint fiber_count;
    float dust_transmission;
    uint raster_radius;
    float density;
    uint2 extent;
    uint fft_side;
    uint sampling;
    int2 origin;
    uint2 cells;
    uint2 screen;
    uint2 end_screen;
    int first_fft_row;
    uint fft_rows;
    uint2 unused;
};
