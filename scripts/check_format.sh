#!/usr/bin/env bash
set -euo pipefail

mode=${1:---worktree}
case "$mode" in
    --staged|--worktree|--fix) ;;
    *) printf '用法：bash scripts/check_format.sh [--staged|--worktree|--fix]\n' >&2; exit 2 ;;
esac

root=$(git rev-parse --show-toplevel)
cd "$root"
scratch_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
scratch=$(mktemp -d "$scratch_parent/simple-engine-format.XXXXXX")
trap '[[ "$scratch" == "$scratch_parent"/simple-engine-format.* ]] && rm -rf -- "$scratch"' EXIT
mkdir "$scratch/tree"

if [[ "$mode" == --staged ]]; then
    git diff --cached --name-only --diff-filter=ACMR -z -- '*.rs' '*.slang' > "$scratch/files"
    # 配置也取自 index，避免未暂存的配置影响提交检查。
    git ls-files -z -- ':(glob)**/.clang-format' ':(glob)**/_clang-format' \
        ':(glob)**/.clang-format-ignore' ':(glob)**/rustfmt.toml' \
        ':(glob)**/.rustfmt.toml' > "$scratch/configs"
    while IFS= read -r -d '' path; do
        mkdir -p "$scratch/tree/$(dirname "$path")"
        git show ":$path" > "$scratch/tree/$path"
    done < "$scratch/configs"
else
    git ls-files --cached --others --exclude-standard -z -- '*.rs' '*.slang' > "$scratch/files"
fi

failed=0
while IFS= read -r -d '' path; do
    if [[ "$mode" == --staged ]]; then
        source="$scratch/tree/$path"
        mkdir -p "$(dirname "$source")"
        git show ":$path" > "$source"
    else
        source="$root/$path"
        [[ -f "$source" ]] || continue
    fi

    case "$path" in
        *.rs)
            command -v rustfmt > /dev/null || { printf '缺少 rustfmt，请安装 Rust rustfmt 组件。\n' >&2; exit 2; }
            # stdin 不递归处理 mod；当前工作区统一使用 Rust 2024。
            (cd "$(dirname "$source")" && rustfmt --edition 2024 --emit stdout < "$source") > "$scratch/formatted"
            ;;
        *.slang)
            command -v clang-format > /dev/null || { printf '缺少 clang-format，请安装 LLVM 并加入 PATH。\n' >&2; exit 2; }
            clang-format --style=file --assume-filename="$source.cs" < "$source" > "$scratch/formatted"
            ;;
    esac

    # Git for Windows 可在检出时转换 CRLF，不将该差异当成格式错误。
    sed 's/\r$//' "$source" > "$scratch/original-lf"
    sed 's/\r$//' "$scratch/formatted" > "$scratch/formatted-lf"
    if ! cmp -s "$scratch/original-lf" "$scratch/formatted-lf"; then
        if [[ "$mode" == --fix ]]; then
            cat "$scratch/formatted" > "$source"
            printf '已格式化：%s\n' "$path"
            continue
        fi
        printf '格式不一致：%s\n' "$path" >&2
        diff -u --label "$path" --label '格式化结果' "$scratch/original-lf" "$scratch/formatted-lf" || true
        failed=1
    fi
done < "$scratch/files"

if (( failed )); then
    printf '请格式化后重新暂存需要提交的内容；钩子不会修改文件或 index。\n' >&2
fi
exit "$failed"
