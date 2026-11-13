# Windows 桌面版安装识别

微软商店和 MSIX 包可能位于 `C:/Program Files/WindowsApps`、`E:/WindowsApps` 等任意应用卷。路径在其他磁盘不代表应用被修改；Windows 的新应用保存位置或应用移动操作都会影响安装卷。

助手按以下顺序进行只读识别：

1. 通过系统 Windows PowerShell 查询当前用户注册的 OpenAI.Codex 包，按版本依次检查真实桌面 EXE。
2. 查询包仓库登记的安装根目录。
3. 查询 Appx 应用卷及本地磁盘顶层的 WindowsApps、Program Files/WindowsApps，仅检查 OpenAI.Codex 包目录。
4. 检查普通桌面安装目录。

备用扫描必须同时存在 Codex 应用清单及桌面程序；不会以残留目录或 `app/resources/codex.exe` 命令行工具作为桌面版已安装依据。输出使用 UTF-8 JSON，支持中文用户名与路径。不修改目录所有权、WindowsApps 权限或包注册。

同一发现结果也用于定位内置模型目录工具。已注册包优先于磁盘上其他版本；最新注册项失效时继续检查可用旧版。

验证：`node scripts/check-codex-detection.cjs`，九种隔离场景。仅使用测试目录和模拟的 Appx/卷/注册表查询，不读取或修改真实 Codex 账户配置。

远端电脑实际识别仍需用新版助手验证。如果 WindowsApps 无访问权限且 Appx/仓库查询也失败，助手不会擅自提权或接管目录。
