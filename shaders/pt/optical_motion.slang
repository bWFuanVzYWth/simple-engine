/// 平面界面的光线微分。三行依次对应两个相机角度和本帧到上一帧的光圈中心位移。
module optical_motion;
namespace pt {
public struct OpticalMotion {
    public var origin : float3x3;
    public var direction : float3x3;
    public var horizontal : float3;
    public var vertical : float3;
    public var valid : bool;
    public var moving : bool;
    public static func camera(ray: float3, displacement: float3)->OpticalMotion {
        var m : OpticalMotion = {};
        m.horizontal = normalize(cross(abs(ray.z) < 0.9 ? float3(0, 0, 1) : float3(0, 1, 0), ray));
        m.vertical = cross(ray, m.horizontal);
        m.direction[0] = m.horizontal;
        m.direction[1] = m.vertical;
        m.origin[2] = displacement;
        m.valid = true;
        return m;
    }
    [mutating]
    public func arrive(distance: float, ray: float3, normal: float3, surface_motion: float3)->void {
        moving = moving || any(surface_motion != 0);
        let denom = dot(normal, ray);
        if (abs(denom) < 1e-5) {
            valid = false;
            return;
        }
        [unroll]
        for (var i = 0; i < 3; ++i) {
            let p = origin[i] + distance * direction[i];
            origin[i] = p - ray * (dot(normal, p - (i == 2 ? surface_motion : float3(0))) / denom);
        }
    }
    [mutating]
    public func scatter(ray: float3, next: float3, normal: float3, eta: float, reflection: bool,
                        refracted: bool)
        ->void {
        if (!reflection && !refracted)
            return;
        let cos_out = abs(dot(next, normal));
        if (!reflection && cos_out < 1e-5) {
            valid = false;
            return;
        }
        let scale = reflection ? 1.0 : eta;
        let slope = reflection ? -2.0 : eta * eta * abs(dot(ray, normal)) / cos_out - eta;
        [unroll]
        for (var i = 0; i < 3; ++i)
            direction[i] = scale * direction[i] + slope * dot(normal, direction[i]) * normal;
    }
    public func previous_ray(camera_ray: float3, target_motion: float3, plane_normal: float3,
                             sky: bool, out ray: float3)
        ->bool {
        let m = sky ? direction : origin;
        let rhs = target_motion - m[2];
        let determinant = dot(cross(m[0], m[1]), plane_normal);
        ray = camera_ray;
        if (!valid || abs(determinant) < 1e-8)
            return false;
        let du = dot(cross(rhs, m[1]), plane_normal) / determinant;
        let dv = dot(cross(m[0], rhs), plane_normal) / determinant;
        ray = normalize(camera_ray + du * horizontal + dv * vertical);
        return all(isfinite(ray));
    }
};
}
