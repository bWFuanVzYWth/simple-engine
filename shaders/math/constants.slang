/// 共享的float32数学常量；不包含颜色编码、采样协议或算法容差。
module constants;

namespace math {
public static const let pi : float = 3.14159265358979323846;
public static const let tau : float = 2.0 * pi;
public static const let frac_1_pi : float = 1.0 / pi;
public static const let frac_1_tau : float = 0.5 * frac_1_pi;
public static const let frac_1_4_pi : float = 0.25 * frac_1_pi;
public static const let frac_pi_2 : float = 0.5 * pi;
public static const let frac_1_sqrt_2 : float = 0.70710678118654752440;
}
