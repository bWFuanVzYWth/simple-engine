# 回归输入

审查后将 `temp/shader-tests/failures` 的 JSON 复制到此目录，使用能说明关注点的文件名。
runner 将每个文件注册为独立的 ignored GPU 测试，直接重放原始输入并检验记录中的性质。
修复后保留输入；不按历史 `actual` 要求继续产生错误结果。

`GPU_TEST_CORPUS` 可指定其他记录目录；不影响种子生成和已有固定反例。
