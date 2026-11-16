//! 同次 Slang 编译的运行核与完整调试副本；只比较，绝不重写发布字节。
use crate::Error;
use std::collections::BTreeSet;

const DEBUG_SET: &str = "NonSemantic.Shader.DebugInfo.100";

fn invalid(reason: &str) -> Error {
    Error::AuditPair(reason.into())
}

fn word(bytes: &[u8], index: usize) -> u32 {
    let start = index * 4;
    u32::from_le_bytes(bytes[start..start + 4].try_into().expect("已校验word范围"))
}

struct Instructions<'a> {
    remaining: &'a [u8],
}

impl<'a> Instructions<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 20
            || !bytes.len().is_multiple_of(4)
            || word(bytes, 0) != 0x0723_0203
            || word(bytes, 4) != 0
        {
            return Err(invalid("SPIR-V header或word对齐无效"));
        }
        Ok(Self {
            remaining: &bytes[20..],
        })
    }
}

impl<'a> Iterator for Instructions<'a> {
    type Item = Result<&'a [u8], Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        let size = (word(self.remaining, 0) >> 16) as usize * 4;
        if size == 0 || size > self.remaining.len() {
            self.remaining = &[];
            return Some(Err(invalid("SPIR-V instruction长度无效")));
        }
        let (instruction, rest) = self.remaining.split_at(size);
        self.remaining = rest;
        Some(Ok(instruction))
    }
}

struct CoreInstructions<'a> {
    instructions: Instructions<'a>,
    debug: BTreeSet<u32>,
}

impl<'a> CoreInstructions<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, Error> {
        let mut debug = BTreeSet::new();
        for instruction in Instructions::new(bytes)? {
            let instruction = instruction?;
            if word(instruction, 0) & 0xffff != 11 {
                continue;
            }
            // OpExtInstImport: result ID, LiteralString。只按规范固定位置读取ID。
            if instruction.len() < 12 {
                return Err(invalid("OpExtInstImport缺少名称"));
            }
            let name = &instruction[8..];
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .ok_or_else(|| invalid("OpExtInstImport名称未终止"))?;
            let name = std::str::from_utf8(&name[..end])
                .map_err(|_| invalid("OpExtInstImport名称不是UTF-8"))?;
            if name.starts_with("NonSemantic.") {
                if name != DEBUG_SET {
                    return Err(invalid(&format!("不能忽略非目标NonSemantic集合：{name}")));
                }
                debug.insert(word(instruction, 1));
            }
        }
        Ok(Self {
            instructions: Instructions::new(bytes)?,
            debug,
        })
    }
}

impl<'a> Iterator for CoreInstructions<'a> {
    type Item = Result<&'a [u8], Error>;

    fn next(&mut self) -> Option<Self::Item> {
        for instruction in self.instructions.by_ref() {
            let instruction = match instruction {
                Ok(instruction) => instruction,
                Err(error) => return Some(Err(error)),
            };
            let excluded = match word(instruction, 0) & 0xffff {
                // 核心debug指令；不删除任何type、constant或Undef声明。
                2..=8 | 317 | 330 => true,
                11 => self.debug.contains(&word(instruction, 1)),
                12 => {
                    if instruction.len() < 20 {
                        return Some(Err(invalid("OpExtInst缺少set/instruction操作数")));
                    }
                    self.debug.contains(&word(instruction, 3))
                }
                _ => false,
            };
            if !excluded {
                return Some(Ok(instruction));
            }
        }
        None
    }
}

/// 校验同一次 Slang separate-debug 编译的主 SPIR-V 与完整调试副本严格同核。
///
/// 两侧须来自受控且启用内部验证的编译器；本函数不是通用SPIR-V合法性验证器。
/// header、ID、类型、常数、Undef、全局接口/装饰、函数指令及次序必须逐word相同。
/// 仅排除标准core debug指令及明确的`NonSemantic.Shader.DebugInfo.100`记录。
/// 不优化、重编号、猜测ID或删除死声明，也不复制/改写输入字节。
/// # Errors
/// 输入截断、核心不一致或出现其它NonSemantic集合（包括DebugPrintf）时返回错误。
/// 配对校验不替代调用方对完整副本的资源、阶段和源码归属审查。
pub fn verify_audit_pair(main: &[u8], audit: &[u8]) -> Result<(), Error> {
    Instructions::new(main)?;
    Instructions::new(audit)?;
    if main[..20] != audit[..20] {
        return Err(invalid("main与完整调试副本的header/核心wordstream不一致"));
    }
    let mut main_core = CoreInstructions::new(main)?;
    let mut audit_core = CoreInstructions::new(audit)?;
    loop {
        match (main_core.next(), audit_core.next()) {
            (None, None) => return Ok(()),
            (Some(left), Some(right)) => {
                if left? != right? {
                    return Err(invalid("main与完整调试副本的核心wordstream不一致"));
                }
            }
            _ => return Err(invalid("main与完整调试副本的核心wordstream长度不一致")),
        }
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
