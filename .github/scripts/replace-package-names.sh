#!/bin/bash
# 完整的包名替换和目录重构脚本

set -euo pipefail

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# 检查参数
if [[ $# -lt 5 ]]; then
    error "用法: $0 <workspace> <new_package> <new_bundle_id> <new_bundle_id_pascal> <new_service_bundle_id> <new_simple_name>"
fi

WORKSPACE="$1"
NEW_PACKAGE="$2"
NEW_BUNDLE_ID="$3"
NEW_BUNDLE_ID_PASCAL="$4"
NEW_SERVICE_BUNDLE_ID="$5"
NEW_SIMPLE_NAME="$6"

log "开始替换包名和标识符..."
log "新包名: $NEW_PACKAGE"
log "新 Bundle ID: $NEW_BUNDLE_ID"

# 设置跨平台 sed 命令
if [[ "$OSTYPE" == "darwin"* ]]; then
    SED_CMD="sed -i ''"
else
    SED_CMD="sed -i"
fi

# 1. 替换所有文件中的包名
replace_in_files() {
    local pattern="$1"
    local replacement="$2"
    
    log "替换模式: $pattern -> $replacement"
    
    # 查找所有相关文件并替换
    find "$WORKSPACE" -type f \( \
        -name "*.rs" -o \
        -name "*.toml" -o \
        -name "*.gradle" -o \
        -name "*.xml" -o \
        -name "*.plist" -o \
        -name "*.pbxproj" -o \
        -name "*.xcconfig" -o \
        -name "*.kt" -o \
        -name "*.scpt" -o \
        -name "CMakeLists.txt" \
    \) -exec grep -l "$pattern" {} \; 2>/dev/null | while read -r file; do
        log "处理文件: $file"
        $SED_CMD "s|$pattern|$replacement|g" "$file"
    done
}

# 执行替换
replace_in_files "com\\.carriez\\.flutter_hbb" "$NEW_PACKAGE"
replace_in_files "com\\.carriez\\.flutterHbb" "$NEW_BUNDLE_ID_PASCAL"
replace_in_files "com\\.carriez\\.rustdesk" "$NEW_BUNDLE_ID"
replace_in_files "com\\.carriez\\.RustDesk_server" "$NEW_SERVICE_BUNDLE_ID"
replace_in_files "com\\.carriez\\.RustDesk" "$NEW_SIMPLE_NAME"

# 2. 移动 Android Kotlin 目录
move_kotlin_dir() {
    local old_pkg="com.carriez.flutter_hbb"
    local new_pkg="$NEW_PACKAGE"
    
    local old_path=$(echo "$old_pkg" | sed 's/\./\//g')
    local new_path=$(echo "$new_pkg" | sed 's/\./\//g')
    
    local old_dir="$WORKSPACE/flutter/android/app/src/main/kotlin/$old_path"
    local new_dir="$WORKSPACE/flutter/android/app/src/main/kotlin/$new_path"
    
    if [[ -d "$old_dir" ]]; then
        log "移动 Kotlin 目录: $old_dir -> $new_dir"
        mkdir -p "$(dirname "$new_dir")"
        if command -v rsync &> /dev/null; then
            rsync -a "$old_dir/" "$new_dir/"
            rm -rf "$old_dir"
        else
            mv "$old_dir" "$new_dir"
        fi
    else
        log "原 Kotlin 目录不存在，可能已被移动"
    fi
}

move_kotlin_dir

log "包名和标识符替换完成！"