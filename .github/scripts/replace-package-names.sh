#!/bin/bash
# 跨平台的包名替换脚本
# 适用于 GitHub Actions 的 Windows、macOS 和 Linux 运行器

set -euo pipefail

# 颜色输出函数
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# 检测操作系统并设置相应工具
detect_os_and_setup() {
    log_info "检测操作系统..."
    
    # 检测 GitHub Actions 运行器环境变量
    if [[ -n "${RUNNER_OS:-}" ]]; then
        OS="$RUNNER_OS"
        log_info "GitHub Actions 环境: $OS"
    else
        # 本地检测
        if [[ "$OSTYPE" == "darwin"* ]]; then
            OS="macOS"
        elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
            OS="Linux"
        elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
            OS="Windows"
        else
            OS="Unknown"
        fi
        log_info "检测到操作系统: $OS"
    fi
    
    # 设置 sed 命令
    case "$OS" in
        "macOS")
            SED_CMD="sed -i.bak"
            NEED_CLEANUP=true
            ;;
        "Windows"|"Linux")
            SED_CMD="sed -i"
            NEED_CLEANUP=false
            ;;
        *)
            log_warn "未知操作系统，使用默认 sed 命令"
            SED_CMD="sed -i"
            NEED_CLEANUP=false
            ;;
    esac
    
    log_info "使用 SED 命令: $SED_CMD"
}

# 安全替换文件函数
safe_replace() {
    local file="$1"
    local pattern="$2"
    local replacement="$3"
    
    if [[ ! -f "$file" ]]; then
        log_warn "文件不存在，跳过: $file"
        return 1
    fi
    
    log_info "处理文件: $file"
    
    # 备份原始文件
    local backup_file="${file}.bak"
    
    if [[ "$NEED_CLEANUP" == "true" ]]; then
        # macOS 方式：使用 sed -i.bak
        $SED_CMD "s|$pattern|$replacement|g" "$file"
        # 清理备份文件
        [[ -f "$backup_file" ]] && rm -f "$backup_file"
    else
        # 其他系统：先备份再替换
        if cp "$file" "$backup_file" 2>/dev/null; then
            $SED_CMD "s|$pattern|$replacement|g" "$file" || {
                log_warn "替换失败，恢复备份: $file"
                mv "$backup_file" "$file"
                return 1
            }
            # 删除备份
            rm -f "$backup_file"
        else
            log_warn "无法创建备份，直接替换: $file"
            $SED_CMD "s|$pattern|$replacement|g" "$file"
        fi
    fi
    
    return 0
}

# 特殊处理 ffi.kt 文件
replace_ffi_kt() {
    local workspace="$1"
    local new_package="$2"
    
    local ffi_kt_file="$workspace/flutter/android/app/src/main/kotlin/ffi.kt"
    
    if [[ ! -f "$ffi_kt_file" ]]; then
        # 尝试在旧位置查找
        ffi_kt_file="$workspace/flutter/android/app/src/main/kotlin/com/carriez/flutter_hbb/ffi.kt"
    fi
    
    if [[ ! -f "$ffi_kt_file" ]]; then
        # 尝试在新位置查找
        local new_kotlin_path=$(echo "$new_package" | tr '.' '/')
        ffi_kt_file="$workspace/flutter/android/app/src/main/kotlin/$new_kotlin_path/ffi.kt"
    fi
    
    if [[ -f "$ffi_kt_file" ]]; then
        log_info "处理 ffi.kt 文件: $ffi_kt_file"
        
        # 替换包名声明
        safe_replace "$ffi_kt_file" "^package com\\.carriez\\.flutter_hbb" "package $new_package" || true
        
        # 替换 import 语句中的包名
        safe_replace "$ffi_kt_file" "import com\\.carriez\\.flutter_hbb\\.RdClipboardManager" "import $new_package.RdClipboardManager" || true
        safe_replace "$ffi_kt_file" "import com\\.carriez\\.flutter_hbb\\." "import $new_package." || true
        
        # 替换类引用
        safe_replace "$ffi_kt_file" "com\\.carriez\\.flutter_hbb" "$new_package" || true
        
        # 检查替换结果
        log_info "ffi.kt 替换后检查:"
        grep -n "package\|import\|carriez" "$ffi_kt_file" | head -10 || true
    else
        log_warn "未找到 ffi.kt 文件"
        # 尝试查找所有可能的 ffi.kt 文件
        find "$workspace/flutter/android/app/src/main/kotlin" -name "ffi.kt" -type f 2>/dev/null | while read -r found_file; do
            log_info "找到 ffi.kt 文件: $found_file"
        done
    fi
}

# 替换包名和标识符
replace_package_names() {
    local workspace="${1:-.}"
    local new_package="$2"
    local new_bundle_id="$3"
    local new_bundle_id_pascal="$4"
    local new_service_bundle_id="$5"
    local new_simple_name="$6"
    
    log_info "开始替换包名和标识符..."
    log_info "工作空间: $workspace"
    log_info "新包名: $new_package"
    
    # 进入工作空间目录
    cd "$workspace" || log_error "无法进入工作空间目录: $workspace"
    
    # 1. 替换 com.carriez.flutter_hbb
    log_info "替换 'com.carriez.flutter_hbb' ..."
    
    local files_to_replace=(
        "Cargo.toml"
        "flutter/android/app/build.gradle"
        "flutter/android/app/src/debug/AndroidManifest.xml"
        "flutter/android/app/src/main/AndroidManifest.xml"
        "flutter/android/app/src/profile/AndroidManifest.xml"
        "flutter/linux/CMakeLists.txt"
    )
    
    for file in "${files_to_replace[@]}"; do
        safe_replace "$file" "com\\.carriez\\.flutter_hbb" "$new_package" || true
    done
    
    # 特殊处理 ffi.kt 文件
    replace_ffi_kt "$workspace" "$new_package"
    
    # Kotlin 源文件 - 使用 find 命令
    local kotlin_dir="flutter/android/app/src/main/kotlin/com/carriez/flutter_hbb"
    if [[ -d "$kotlin_dir" ]]; then
        log_info "处理 Kotlin 目录: $kotlin_dir"
        find "$kotlin_dir" -name "*.kt" -type f | while read -r ktfile; do
            log_info "  处理: $ktfile"
            # 替换包声明
            safe_replace "$ktfile" "^package com\\.carriez\\.flutter_hbb" "package $new_package" || true
            # 替换所有出现的旧包名
            safe_replace "$ktfile" "com\\.carriez\\.flutter_hbb" "$new_package" || true
        done
    fi
    
    # 2. 替换 com.carriez.flutterHbb (PascalCase)
    log_info "替换 'com.carriez.flutterHbb' ..."
    
    files_to_replace=(
        "flutter/ios/exportOptions.plist"
        "flutter/ios/Runner/GoogleService-Info.plist"
        "flutter/ios/Runner.xcodeproj/project.pbxproj"
        "flutter/macos/Runner/Configs/AppInfo.xcconfig"
    )
    
    for file in "${files_to_replace[@]}"; do
        safe_replace "$file" "com\\.carriez\\.flutterHbb" "$new_bundle_id_pascal" || true
    done
    
    # 3. 替换 com.carriez.rustdesk
    log_info "替换 'com.carriez.rustdesk' ..."
    
    files_to_replace=(
        "flutter/ios/Runner/Info.plist"
        "flutter/macos/Runner/Info.plist"
        "flutter/macos/Runner.xcodeproj/project.pbxproj"
        "src/platform/macos.rs"
        "src/platform/privileges_scripts/agent.plist"
        "src/platform/privileges_scripts/daemon.plist"
        "src/platform/privileges_scripts/install.scpt"
    )
    
    for file in "${files_to_replace[@]}"; do
        safe_replace "$file" "com\\.carriez\\.rustdesk" "$new_bundle_id" || true
    done
    
    # 4. 替换服务标识 com.carriez.RustDesk_server
    log_info "替换服务标识 'com.carriez.RustDesk_server' ..."
    
    # 特殊处理 XPC_SERVICE_NAME
    safe_replace "src/common.rs" "XPC_SERVICE_NAME = \"com\\.carriez\\.RustDesk_server\"" "XPC_SERVICE_NAME = \"$new_service_bundle_id\"" || true
    
    files_to_replace=(
        "src/platform/privileges_scripts/agent.plist"
        "src/platform/privileges_scripts/daemon.plist"
        "src/platform/privileges_scripts/install.scpt"
        "src/platform/privileges_scripts/uninstall.scpt"
        "src/platform/privileges_scripts/update.scpt"
    )
    
    for file in "${files_to_replace[@]}"; do
        safe_replace "$file" "com\\.carriez\\.RustDesk_server" "$new_service_bundle_id" || true
    done
    
    # 5. 替换 com.carriez.RustDesk (简化版本)
    log_info "替换 'com.carriez.RustDesk' ..."
    safe_replace "src/platform/privileges_scripts/install.scpt" "com\\.carriez\\.RustDesk" "$new_simple_name" || true
    
    # 清理备份文件 (macOS)
    if [[ "$NEED_CLEANUP" == "true" ]]; then
        log_info "清理备份文件..."
        find . -name "*.bak" -type f -delete 2>/dev/null || true
    fi
    
    # 6. 移动 Android Kotlin 目录
    log_info "重构 Android Kotlin 目录..."
    
    local old_kotlin_dir="flutter/android/app/src/main/kotlin/com/carriez/flutter_hbb"
    local new_kotlin_path=$(echo "$new_package" | tr '.' '/')
    local new_kotlin_dir="flutter/android/app/src/main/kotlin/$new_kotlin_path"
    
    log_info "旧目录: $old_kotlin_dir"
    log_info "新目录: $new_kotlin_dir"
    
    if [[ -d "$old_kotlin_dir" ]]; then
        log_info "移动 Kotlin 目录..."
        
        # 创建父目录
        mkdir -p "$(dirname "$new_kotlin_dir")"
        
        # 移动目录
        if command -v rsync >/dev/null 2>&1; then
            rsync -a "$old_kotlin_dir/" "$new_kotlin_dir/"
            rm -rf "$old_kotlin_dir"
        else
            mv "$old_kotlin_dir" "$new_kotlin_dir"
        fi
        
        log_info "Kotlin 目录移动完成"
        
        # 检查新目录中的文件
        log_info "新目录内容:"
        find "$new_kotlin_dir" -name "*.kt" -type f | head -5 | while read file; do
            log_info "  - $file"
            # 检查包名是否正确
            if grep -q "^package $new_package" "$file"; then
                log_info "    ✓ 包名正确"
            else
                log_warn "    ⚠ 包名可能需要更新"
                head -5 "$file" | grep -i "package\|import" || true
            fi
        done
    else
        log_warn "原 Kotlin 目录不存在，跳过移动"
        log_info "当前 kotlin 目录内容:"
        ls -la "flutter/android/app/src/main/kotlin/" 2>/dev/null || true
    fi
    
    log_info "包名和标识符替换完成！"
}

# 验证替换结果
verify_replacements() {
    local workspace="$1"
    local new_package="$2"
    
    log_info "=== 验证替换结果 ==="
    
    cd "$workspace" || return
    
    # 检查是否有未替换的旧包名
    log_info "1. 检查未替换的旧包名..."
    
    # 检查 Kotlin 文件
    find "flutter/android/app/src/main/kotlin" -name "*.kt" -type f 2>/dev/null | while read -r ktfile; do
        if grep -q "com\\.carriez\\.flutter_hbb" "$ktfile"; then
            log_warn "文件 $ktfile 包含未替换的旧包名:"
            grep -n "com\\.carriez\\.flutter_hbb" "$ktfile" | head -3
        fi
    done
    
    # 检查 ffi.kt 文件
    local ffi_kt_files=(
        "flutter/android/app/src/main/kotlin/ffi.kt"
        "flutter/android/app/src/main/kotlin/com/carriez/flutter_hbb/ffi.kt"
    )
    
    local new_kotlin_path=$(echo "$new_package" | tr '.' '/')
    ffi_kt_files+=("flutter/android/app/src/main/kotlin/$new_kotlin_path/ffi.kt")
    
    for ffi_file in "${ffi_kt_files[@]}"; do
        if [[ -f "$ffi_file" ]]; then
            log_info "检查 ffi.kt: $ffi_file"
            if grep -q "com\\.carriez\\.flutter_hbb" "$ffi_file"; then
                log_warn "  ⚠ 包含未替换的旧包名"
                grep -n "com\\.carriez\\.flutter_hbb" "$ffi_file"
            else
                log_info "  ✓ 已正确替换"
            fi
            break
        fi
    done
    
    log_info "2. 检查 RdClipboardManager 引用..."
    find "flutter/android/app/src/main/kotlin" -name "*.kt" -type f 2>/dev/null -exec grep -l "RdClipboardManager" {} \; | while read -r file; do
        log_info "  在文件中找到 RdClipboardManager: $file"
        grep -n "RdClipboardManager" "$file" | head -2
    done
    
    log_info "验证完成"
}

# 主函数
main() {
    # 检测参数
    if [[ $# -lt 6 ]]; then
        log_error "用法: $0 <workspace> <new_package> <new_bundle_id> <new_bundle_id_pascal> <new_service_bundle_id> <new_simple_name>"
        log_error "示例: $0 . com.example.app com.example.app com.example.app com.example.RustDesk_server com.example.RustDesk"
        exit 1
    fi
    
    local workspace="$1"
    local new_package="$2"
    local new_bundle_id="$3"
    local new_bundle_id_pascal="$4"
    local new_service_bundle_id="$5"
    local new_simple_name="$6"
    
    # 检测操作系统
    detect_os_and_setup
    
    # 替换包名
    replace_package_names "$workspace" "$new_package" "$new_bundle_id" "$new_bundle_id_pascal" "$new_service_bundle_id" "$new_simple_name"
    
    # 验证替换结果
    verify_replacements "$workspace" "$new_package"
    
    log_info "脚本执行完成！"
}

# 执行主函数
main "$@"