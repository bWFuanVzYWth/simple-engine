# 后续资源管理

- 分离不可变 RT program/SBT 与场景描述符，资源重绑复用相同 shader 身份的 program。
- 有界批构建 AS，复用 upload/scratch/AS backing；按 GPU 完成证明回收，记录并发峰值与 allocation 数量。
- 静态 BLAS 增加预算内 compaction，验收构建/copy成本、常驻字节及旧版本最后消费者。
- 补充资源 heap、驻留与完整工作集诊断；区分 DEVICE_LOCAL 属性和实际驻留，按并发与在途峰值评估预算。

场景灯目录的失效判定属于上层；通用后端不解释几何编辑是否改变发光面。以上均为待测成本改进，不作为当前正确性缺陷或性能收益承诺。
