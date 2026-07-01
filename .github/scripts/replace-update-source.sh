#!/bin/bash
# 构建时注入"自建 Gitea 更新源"，使客户端从发布站检查/下载更新。
#
# 设计目标：lib.rs / common.rs 保持上游原版（merge 上游 tag 时零冲突），
# 所有更新源改动均在本脚本构建时注入，与 replace-servers-keys.sh 同模式。
#
# 注入内容：
#   1. hbb_common/src/lib.rs
#      把 version_check_request 的端点从 https://api.rustdesk.com/version/latest
#      替换为自建 Gitea 兼容 JSON 的 raw 地址（latest.json）。
#      该 JSON 返回上游期望的 {"url":".../releases/tag/<ver>"} 格式，
#      因此 do_check_software_update() 函数体完全不用改。
#      - GET 还是 POST：上游用 POST，Gitea raw 文件对 POST 同样返回内容，
#        且不带 body 时 Gitea 也兼容，故无需改 HTTP 方法。
#      - 解析：上游用 VersionCheckResponse{url} 解析，与 latest.json 格式一致。
#
#   2. src/common.rs
#      把 is_custom_client() 的实现改为恒返回 false，解除品牌版
#      （已改包名，get_app_name() != "RustDesk"）的自动更新检查阻断，
#      使 check_software_update() 不再因 is_custom_client() 提前 return。
#
# 用法: replace-update-source.sh <workspace> <update_json_url> <repo_html_url>
#   update_json_url : latest.json 的 raw URL（兼容格式 {url:.../tag/<ver>}）
#   repo_html_url   : 发布站 HTML 地址前缀，用于拼 latest.json 中的 url 字段

set -euo pipefail

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

if [[ $# -lt 2 ]]; then
    log_error "用法: $0 <workspace> <update_json_url> [repo_html_url]"
    exit 1
fi

WORKSPACE="$1"
UPDATE_JSON_URL="$2"

# 非空校验：为空会注入空 URL（perl 替换成 ""），且 grep -q "" 恒真导致校验失效。
if [[ -z "${UPDATE_JSON_URL}" ]]; then
    log_error "UPDATE_JSON_URL 为空，请检查 GitHub Secret 是否配置"
    exit 1
fi

# 检测操作系统选择 sed 风格
detect_os() {
    if [[ -n "${RUNNER_OS:-}" ]]; then
        echo "$RUNNER_OS"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        echo "macOS"
    else
        echo "Linux"
    fi
}

main() {
    local os sed_cmd
    os=$(detect_os)
    case "$os" in
        "macOS") sed_cmd="sed -i.bak" ;;
        *) sed_cmd="sed -i" ;;
    esac

    cd "$WORKSPACE" || log_error "无法进入工作空间: $WORKSPACE"
    log_info "注入更新源: $UPDATE_JSON_URL"

    # --- 1. 替换 hbb_common/src/lib.rs 的版本检查端点 ---
    local lib_rs="libs/hbb_common/src/lib.rs"
    if [[ -f "$lib_rs" ]]; then
        log_info "替换 lib.rs 版本检查端点..."
        # 上游锚点: const URL: &str = "https://api.rustdesk.com/version/latest";
        # 用 perl 而非 sed：替换串中含 '&'（&str），sed 会把 '&' 当作"整个匹配"引用，
        # 而 perl 对替换串字面量更可控，且 macOS/Linux 行为一致。
        if perl -i -pe 's|const URL: &str = "https://api\.rustdesk\.com/version/latest";|const URL: \&str = "'"${UPDATE_JSON_URL}"'";|' "$lib_rs"; then
            :
        fi

        # 校验：确认上游锚点已消失（不能用 grep URL 空串恒真）
        if ! grep -q 'api\.rustdesk\.com/version/latest' "$lib_rs"; then
            log_info "✅ lib.rs 端点已注入: ${UPDATE_JSON_URL}"
        else
            log_error "lib.rs 端点注入失败（上游锚点可能已变化，请检查 version_check_request）"
        fi
    else
        log_error "未找到 $lib_rs"
    fi

    # --- 2. 解除 common.rs 中 is_custom_client() 阻断 ---
    local common_rs="src/common.rs"
    if [[ -f "$common_rs" ]]; then
        log_info "解除 common.rs 的 is_custom_client() 更新阻断..."
        # 上游锚点（单行实现）:
        #   pub fn is_custom_client() -> bool {
        #       get_app_name() != "RustDesk"
        #   }
        # 把函数体替换为恒 false，使 check_software_update() 不再提前 return。
        # 替换串无特殊字符，sed 安全。
        $sed_cmd 's|get_app_name() != "RustDesk"|false|' "$common_rs"

        if grep -q 'pub fn is_custom_client() -> bool {' "$common_rs"; then
            log_info "✅ is_custom_client() 已改为恒 false（解除阻断）"
        else
            log_warn "未找到 is_custom_client 函数锚点（可能上游已重构），跳过"
        fi
    else
        log_error "未找到 $common_rs"
    fi

    # 清理 macOS sed 备份
    if [[ "$os" == "macOS" ]]; then
        find . -name "*.bak" -type f -delete 2>/dev/null || true
    fi

    log_info "更新源注入完成！"
}

main "$@"
