export const messages: Readonly<Record<string, string>> = {
  "Build snapshot": "构建快照",
  "Run an install script once, then reuse the resulting provider snapshot.":
    "运行一次安装脚本，然后重复使用生成的提供商快照。",
  Provider: "提供商",
  "Base snapshot": "基础快照",
  "Provider default": "提供商默认值",
  "Source revision": "源修订版本",
  "Select and save a named Organization provider account before building.":
    "构建前，请选择并保存组织中已命名的提供商账户。",
  "Save your configuration changes before building.": "构建前，请保存配置更改。",
  "Install script": "安装脚本",
  "Verification and build access": "验证与构建访问权限",
  "Verification command (optional)": "验证命令（可选）",
  "Metadata file (optional)": "元数据文件（可选）",
  "Capture up to 16 KiB of non-secret software notes written by your script.":
    "捕获脚本写入的最多 16 KiB 的非机密软件说明。",
  "Total build timeout (seconds)": "构建总超时（秒）",
  "Build network": "构建网络",
  "Loading…": "加载中…",
  "Build result": "构建结果",
  Step: "步骤",
  "Resource cleanup is pending and will be retried.": "资源清理尚未完成，将重试。",
  "Captured software notes": "捕获的软件说明",
  "Build log": "构建日志",
  "Waiting for output…": "等待输出…",
  "Save new revision": "保存新修订版本",
  "Discard snapshot": "丢弃快照",
  "The source revision changed. Build again from the current template.":
    "源修订版本已更改。请从当前模板重新构建。",
  Queued: "排队中",
  Running: "运行中",
  Ready: "就绪",
  Failed: "失败",
  Cancelled: "已取消",
  Saved: "已保存",
  Discarded: "已丢弃",
  "Starting build compute": "正在启动构建环境",
  "Running install script": "正在运行安装脚本",
  "Verifying installation": "正在验证安装",
  "Creating snapshot": "正在创建快照",
  "Verifying fresh snapshot": "正在验证从快照启动的新环境",
  "Cancellation requested": "已请求取消",
  "Cancel build": "取消构建",
  "Built snapshot": "已构建的快照",
  "Sandbox Template sections": "沙箱模板分区",
  Configuration: "配置",
  "Snapshot builds currently support Daytona. Other template targets remain configurable.":
    "快照构建目前支持 Daytona。其他模板目标仍可配置。",
  "Runs in fresh isolated build compute. Launch bootstrap commands stay separate. Do not put credentials in scripts.":
    "在全新的隔离构建环境中运行。启动命令保持独立。请勿将凭据放入脚本。",
  "Runs after installation and again on a fresh sandbox from the snapshot. Only what this command checks is verified.":
    "安装后运行，并在从快照启动的新沙箱中再次运行。仅验证此命令检查的内容。",
  "Allow package sources during installation without expanding the finished sandbox’s runtime policy.":
    "在安装期间允许访问软件包源，而不扩大最终沙箱的运行时策略。",
  "Build compute and snapshot storage are billed to the selected provider account. Runtime credentials and private workspace data are not attached.":
    "构建计算和快照存储费用由所选提供商账户承担。不会附加运行时凭据或私有工作区数据。",
  "Author-supplied reference data. These notes do not grant capabilities or change runtime policy.":
    "作者提供的参考数据。这些说明不授予能力，也不更改运行时策略。",
  "Discard this unused provider snapshot? Applied template revisions are kept.":
    "丢弃此未使用的提供商快照？已应用的模板修订版本将保留。",
  "Older builds": "更早的构建",
  "Reuse script": "重复使用脚本",
};
