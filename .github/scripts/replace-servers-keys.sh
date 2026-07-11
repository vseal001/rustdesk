#!/bin/bash
# 跨平台的服务器和密钥替换脚本

set -euo pipefail

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# 检测操作系统
detect_os() {
    if [[ -n "${RUNNER_OS:-}" ]]; then
        echo "$RUNNER_OS"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        echo "macOS"
    elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo "Linux"
    elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
        echo "Windows"
    else
        echo "Unknown"
    fi
}

main() {
    if [[ $# -lt 3 ]]; then
        log_error "用法: $0 <workspace> <rendezvous_server> <rs_pub_key> <api_server>"
        exit 1
    fi
    
    local workspace="$1"
    local rendezvous_server="$2"
    local rs_pub_key="$3"
    local api_server="$4"
    
    log_info "开始替换服务器和密钥..."
    log_info "工作空间: $workspace"
    
    cd "$workspace" || log_error "无法进入工作空间"
    
    # 设置 sed 命令
    local os=$(detect_os)
    local sed_cmd
    
    case "$os" in
        "macOS") sed_cmd="sed -i.bak" ;;
        *) sed_cmd="sed -i" ;;
    esac
    
    log_info "操作系统: $os"
    log_info "SED 命令: $sed_cmd"
    
    # 替换 RENDEZVOUS_SERVERS
    log_info "替换 RENDEZVOUS_SERVERS..."
    if [[ -f "libs/hbb_common/src/config.rs" ]]; then
        $sed_cmd '/RENDEZVOUS_SERVERS.*rs-ny\.rustdesk\.com/s/rs-ny\.rustdesk\.com/'"$rendezvous_server"'/' "libs/hbb_common/src/config.rs"
    fi
    
    # 替换 RS_PUB_KEY
    log_info "替换 RS_PUB_KEY..."
    if [[ -f "libs/hbb_common/src/config.rs" ]]; then
        $sed_cmd '/RS_PUB_KEY/s/OeVuKk5nlHiXp+APNn0Y3pC1Iwpwn44JGqrQCsWqmBw=/'"$rs_pub_key"'/' "libs/hbb_common/src/config.rs"
    fi
    
    # 替换 API 服务器
    log_info "替换 API 服务器..."
    if [[ -f "src/common.rs" ]]; then
        $sed_cmd 's|https://admin.rustdesk.com|'"$api_server"'|g' "src/common.rs"
    fi
    
    # 清理备份文件 (macOS)
    if [[ "$os" == "macOS" ]]; then
        log_info "清理备份文件..."
        find . -name "*.bak" -type f -delete 2>/dev/null || true
    fi
    
    log_info "服务器和密钥替换完成！"
}

main "$@"