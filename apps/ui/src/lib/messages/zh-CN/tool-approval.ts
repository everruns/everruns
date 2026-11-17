// zh-CN product copy for tool approval: Settings → Approvals, My approvals, the agent's
// approval settings, the session mode switch and review marks in chat
// (knowledge/execution/tool-approval.md).
export const messages: Readonly<Record<string, string>> = {
  "A reviewer answers first, asks you when unsure": "审核者先作答，拿不准时询问你",
  "A reviewer answers routine calls; you get the unclear ones":
    "审核者处理常规调用，不明确的交给你",
  Action: "操作",
  "Add a note": "添加备注",
  "Add rule": "添加规则",
  "Agent default": "智能体默认",
  "Allow at least one mode.": "请至少允许一种模式。",
  "Allowed modes": "允许的模式",
  "Always ask": "始终询问",
  "Applies from the next tool call.": "从下一次工具调用起生效。",
  "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.":
    "适用于你发起的会话。在组织允许的范围内，你仍可在会话标题栏切换单个会话。",
  "Approval mode": "审批模式",
  "Approval mode: {mode}": "审批模式：{mode}",
  "Approvals: {mode}": "审批：{mode}",
  "Asked a person": "已询问人员",
  "Asks before any change": "任何更改前都询问",
  "Asks before risky calls": "有风险的调用前询问",
  "Auto review is paused after several blocked calls. Every call now waits for you until you approve one.":
    "多次调用被拦截后，自动审核已暂停。在你批准一次调用之前，每个调用都会等待你。",
  "Auto shows only when the organization allows it.": "仅当组织允许时才显示自动模式。",
  "Auto-allowed": "已自动允许",
  "Choose which approval modes agents may use and set rules for every agent":
    "选择智能体可使用的审批模式，并为所有智能体设置规则",
  "Could not change the mode. You may not be allowed to manage this session.":
    "无法更改模式。你可能无权管理此会话。",
  "Could not load recent reviews.": "无法加载最近的审核。",
  "Could not save.": "无法保存。",
  "Each agent's own mode": "每个智能体自己的模式",
  "How agents in this organization get a go-ahead before they use a tool.":
    "此组织中的智能体在使用工具前如何获得许可。",
  "How tool calls in your own sessions get approved.": "你自己会话中的工具调用如何获批。",
  "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.":
    "智能体和人员可选择的模式。关闭的模式会提升为下一个更谨慎的允许模式。",
  "My approvals": "我的审批",
  "My extra rules": "我的额外规则",
  "Never asks": "从不询问",
  "No auto reviews yet.": "暂无自动审核。",
  Normal: "常规",
  "Not available: this agent's manager turned approvals on": "不可用：此智能体的管理者已开启审批",
  "Note {number}": "备注 {number}",
  "Notes for the reviewer": "给审核者的备注",
  "Only organization owners and admins can change these settings.":
    "只有组织所有者和管理员可以更改这些设置。",
  "Only the prod-* clusters are production.": "只有 prod-* 集群是生产环境。",
  "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.":
    "个人规则只能增加谨慎程度：始终询问或从不。它们适用于智能体代表你发起的调用。",
  "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.":
    "自动审核者为每个智能体阅读的简单语句：你的域名、你的代码仓库、哪些系统是生产环境。",
  "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.":
    "自动审核者阅读的简单语句，例如哪些仓库属于你，或什么算生产环境。",
  Protective: "保护",
  "Recent reviews": "最近的审核",
  "Remove note {number}": "删除备注 {number}",
  "Remove rule for {tool}": "删除 {tool} 的规则",
  Rules: "规则",
  "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.":
    "规则在所有模式下生效；最严格的匹配优先。工具名可使用 * 作为通配符。",
  "Rules for every agent": "所有智能体的规则",
  "Start my sessions in": "我的会话起始模式",
  "The approval mode your sessions start in and your own extra rules":
    "你的会话起始审批模式和你自己的额外规则",
  "The last calls the auto reviewer decided on, newest first.":
    "自动审核者最近决定的调用，最新的在前。",
  "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.":
    "这些规则适用于组织内所有智能体和所有模式。规则冲突时，最谨慎的优先。",
  "Tool name or pattern": "工具名或模式",
  "Trust notes for the reviewer": "给审核者的信任备注",
  "Unable to load approval settings.": "无法加载审批设置。",
  Verdict: "结论",
  When: "时间",
  "Why you're asked:": "询问你的原因：",
  "Your own default, else the agent's mode ({mode})":
    "你自己的默认值，否则为智能体的模式（{mode}）",
  "github.com/acme is ours; pushing branches there is routine.":
    "github.com/acme 是我们的；向其推送分支属于常规操作。",
  "{mode} (agent default)": "{mode}（智能体默认）",
};
