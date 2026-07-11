# 自签名代码签名脚本（用于 GitHub Actions Windows runner）。
#
# 用途：在 SIGN_BASE_URL 未配置（无远程签名服务）时，用自签名证书本地签名
# Windows 产物（exe/dll/sys/msi），去掉"未知发布者"红框，安装时显示发布者名称。
#
# 工作原理：
#   1. 用 PowerShell New-SelfSignedCertificate 生成代码签名证书（每次构建临时生成，
#      仅用于本次构建签名，不持久化——自签名证书不会被 Windows 信任，用户首次
#      安装时需手动信任发布者，之后不再提示）。
#   2. 导出为 .pfx，用 signtool.exe 对目标目录下所有 exe/dll/msi/sys 签名。
#
# 局限：自签名证书不被 Windows/SmartScreen 信任，首次安装会有"未知发布者"提示，
# 用户点"仍要运行"即可；不像正式 CA 证书那样静默通过。适合内部使用/测试。
#
# 用法（PowerShell）:
#   .\sign-self-signed.ps1 -Dir <要签名的目录> [-Subject <发布者名称>] [-TimestampServer <时间戳URL>]
#
# 示例:
#   .\sign-self-signed.ps1 -Dir .\SignOutput -Subject "RustDesk Custom Build"
#   .\sign-self-signed.ps1 -Dir .\rustdesk -Subject "vseal001"

param(
    [Parameter(Mandatory = $true)]
    [string]$Dir,

    # 发布者名称（显示在签名证书的"颁发给"和安装界面）
    [string]$Subject = "RustDesk Custom Build",

    # 时间戳服务器（让签名时间被锚定，证书过期后旧签名仍有效）
    [string]$TimestampServer = "http://timestamp.digicert.com",

    # 要签名的扩展名（逗号分隔）
    [string]$Extensions = ".exe,.dll,.sys,.msi,.msix,.appx"
)

$ErrorActionPreference = "Stop"

Write-Host "=== 自签名代码签名 ===" -ForegroundColor Cyan
Write-Host "目标目录: $Dir"
Write-Host "发布者: $Subject"

if (-not (Test-Path $Dir)) {
    Write-Host "[WARN] 目录不存在: $Dir，跳过签名" -ForegroundColor Yellow
    exit 0
}

# --- 1. 定位 signtool.exe ---
$signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" |
    Select-Object -Last 1 -ExpandProperty FullName -ErrorAction SilentlyContinue
if (-not $signtool) {
    Write-Host "[WARN] 未找到 signtool.exe，跳过签名" -ForegroundColor Yellow
    exit 0
}
Write-Host "signtool: $signtool"

# --- 2. 生成自签名代码签名证书 ---
Write-Host "生成自签名证书..."
$cert = New-SelfSignedCertificate `
    -Subject "CN=$Subject" `
    -Type CodeSigningCert `
    -KeyUsage DigitalSignature `
    -FriendlyName $Subject `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -NotAfter (Get-Date).AddYears(1)

if (-not $cert) {
    Write-Host "[ERROR] 证书生成失败" -ForegroundColor Red
    exit 1
}
Write-Host "证书指纹: $($cert.Thumbprint)"

# 导出 .pfx（带私钥，signtool 需要）
$pfxPath = Join-Path $env:TEMP "self-signed-codesign.pfx"
$pwd = ConvertTo-SecureString -String "temp-sign-pwd-123" -Force -AsPlainText
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $pwd | Out-Null

# 把证书放到"受信任的根"和"受信任的发布者"，让本机验证签名通过（仅本次构建环境）
$rootStore = Get-Item "Cert:\LocalMachine\Root"
$publisherStore = Get-Item "Cert:\LocalMachine\TrustedPublisher"
$rootStore.Open("ReadWrite")
$publisherStore.Open("ReadWrite")
$rootStore.Add($cert)
$publisherStore.Add($cert)
$rootStore.Close()
$publisherStore.Close()
Write-Host "证书已加入受信任存储（本次构建环境）" -ForegroundColor Green

# --- 3. 遍历目录签名所有目标文件 ---
$extList = $Extensions -split ","
$files = Get-ChildItem -Path $Dir -Recurse -File | Where-Object { $extList -contains $_.Extension.ToLower() }
Write-Host "待签名文件数: $($files.Count)"

$success = 0
$failed = 0
foreach ($file in $files) {
    Write-Host "签名: $($file.FullName)"
    & $signtool sign /f $pfxPath /p "temp-sign-pwd-123" `
        /tr $TimestampServer /td sha256 /fd sha256 `
        $file.FullName 2>&1 | ForEach-Object { Write-Host "  $_" }
    if ($LASTEXITCODE -eq 0) {
        $success++
    } else {
        $failed++
        Write-Host "  [WARN] 签名失败: $($file.Name)" -ForegroundColor Yellow
    }
}

# --- 4. 清理临时证书（不留痕） ---
Remove-Item $pfxPath -Force -ErrorAction SilentlyContinue
Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -Force -ErrorAction SilentlyContinue

Write-Host "=== 签名完成: 成功 $success / 失败 $failed ===" -ForegroundColor Cyan
if ($failed -gt 0) {
    Write-Host "[WARN] 有 $failed 个文件签名失败（非致命，继续构建）" -ForegroundColor Yellow
}
exit 0
