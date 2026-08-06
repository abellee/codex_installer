import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Bot,
  Check,
  CheckCircle2,
  CircleHelp,
  Code2,
  Copy,
  Download,
  ExternalLink,
  FlaskConical,
  Gauge,
  Headphones,
  KeyRound,
  LoaderCircle,
  Maximize2,
  PackageCheck,
  Play,
  RefreshCw,
  Settings2,
  ShieldCheck,
  Sparkles,
  TerminalSquare,
  Wrench,
  X,
  XCircle,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import llmfreePreview from "./assets/llmfree-preview.png";
import codexPlusSuppliers from "./assets/tutorials/codexpp-01-suppliers.png";
import codexPlusAdd from "./assets/tutorials/codexpp-02-add.png";
import codexPlusApiMode from "./assets/tutorials/codexpp-03-api-mode.png";
import codexPlusApiFields from "./assets/tutorials/codexpp-04-api-fields.png";
import codexPlusSave from "./assets/tutorials/codexpp-05-save.png";
import ccSwitchCodex from "./assets/tutorials/ccswitch-01-codex.png";
import ccSwitchCustom from "./assets/tutorials/ccswitch-02-custom.png";
import ccSwitchKey from "./assets/tutorials/ccswitch-03-key.png";
import ccSwitchEndpoint from "./assets/tutorials/ccswitch-04-endpoint.png";

type Screen = "choose" | "installing" | "complete" | "failed";
type Companion = "codex-plus-plus" | "cc-switch";

type ProgressEvent = {
  runId: number;
  percent: number;
  phase: string;
  title: string;
  detail: string;
  component: string;
};

type InstallResult = {
  codexVersion: string;
  companionName: string;
  installDirectory: string;
};

const QQ_NUMBER = "751077517";
const LLMFREE_URL = "https://www.llmfree.work";

const companions = [
  {
    id: "codex-plus-plus" as const,
    name: "Codex++",
    icon: Sparkles,
    badge: "新手推荐",
    summary: "只为 Codex 服务，重点是把配置步骤变简单。",
    detail: "适合第一次使用 AI 编程工具的人。界面更直接，需要理解的选项更少，装好后跟着提示配置即可。",
    points: ["只管理 Codex", "配置步骤更少", "上手更轻松"],
  },
  {
    id: "cc-switch" as const,
    name: "CC Switch",
    icon: Settings2,
    badge: "多工具用户",
    summary: "可以管理多种常用 Agent，但需要认识更多配置项。",
    detail: "适合同时使用 Codex、Claude Code、Gemini CLI 等工具的人。功能更全面，相应的配置也更多。",
    points: ["支持多种 Agent", "切换能力更丰富", "需要更多配置"],
  },
];

const tutorialContent = {
  "codex-plus-plus": {
    name: "Codex++",
    intro: "Codex++ 不需要先创建其他配置。打开管理工具后，直接进入“供应商配置”添加供应商。",
    slides: [
      { image: codexPlusSuppliers, title: "进入供应商配置", body: "打开 Codex++ 管理工具，在左侧点击“供应商配置”。所有 API 服务都在这里管理。" },
      { image: codexPlusAdd, title: "添加供应商", body: "点击供应商列表上方的“添加供应商”，进入新供应商页面。" },
      { image: codexPlusApiMode, title: "选择纯 API", body: "如果使用中转站提供的地址和密钥，在“接入模式”中选择“纯 API”。" },
      { image: codexPlusApiFields, title: "填写连接信息", body: "填写名称、Base URL、API Key 和配置模型。Base URL 与模型名称以服务商提供的信息为准。" },
      { image: codexPlusSave, title: "同步模型并保存", body: "选择 Responses API，点击“从上游获取”载入模型，确认模型列表后点击左上角“保存”。" },
    ],
    note: "如果你使用 LLM Free，可在网站控制台复制 API 地址和密钥；建议先用试用额度确认配置可用。",
  },
  "cc-switch": {
    name: "CC Switch",
    intro: "CC Switch 可以管理多种 Agent。配置 Codex 时要先进入 Codex 对应区域，避免把密钥填到其他工具里。",
    slides: [
      { image: ccSwitchCodex, title: "切换到 Codex", body: "点击顶部的 Codex 图标，再点击右上角加号。CC Switch 支持多种 Agent，所以一定先确认当前是 Codex。" },
      { image: ccSwitchCustom, title: "添加自定义供应商", body: "保持“Codex 供应商”页，选择“自定义配置”，然后点击右下角“添加”。" },
      { image: ccSwitchKey, title: "填写名称和 API Key", body: "供应商名称只用于自己识别。API Key 是私密凭证，只粘贴到工具里，不要发给其他人。" },
      { image: ccSwitchEndpoint, title: "同步并选择默认模型", body: "填写请求地址后，点击默认模型右侧的向下箭头同步上游模型；同步完成后点击新出现的下拉箭头，在列表中选择默认模型。" },
    ],
    note: "CC Switch 功能更多。第一次只配置 Codex 即可，确认能用后再逐步添加 Claude Code 或 Gemini CLI。",
  },
};

const steps = [
  { label: "选择组件", icon: PackageCheck },
  { label: "下载与安装", icon: Download },
  { label: "开始使用", icon: CheckCircle2 },
];

function isTauriRuntime() {
  return "__TAURI_INTERNALS__" in window;
}

function App() {
  const userAgent = navigator.userAgent.toLowerCase();
  const isMacosRuntime = isTauriRuntime() && (userAgent.includes("macintosh") || userAgent.includes("mac os x"));
  const isPreviewMode = !isTauriRuntime() || isMacosRuntime;
  const [screen, setScreen] = useState<Screen>("choose");
  const [companion, setCompanion] = useState<Companion>("codex-plus-plus");
  const [progress, setProgress] = useState<ProgressEvent>({
    runId: 0,
    percent: 0,
    phase: "ready",
    title: "准备开始",
    detail: "确认选择后，我们会自动完成剩余步骤。",
    component: "环境检查",
  });
  const [activity, setActivity] = useState<ProgressEvent[]>([]);
  const [installResult, setInstallResult] = useState<InstallResult | null>(null);
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [tutorial, setTutorial] = useState<Companion | null>(null);
  const installationAttempt = useRef(0);

  const recordActivity = (item: ProgressEvent) => {
    setActivity((previous) => {
      const key = `${item.component}:${item.phase}`;
      const withoutDuplicate = previous.filter((entry) => `${entry.component}:${entry.phase}` !== key);
      return [...withoutDuplicate, item].slice(-5);
    });
  };

  const currentStep = screen === "choose" ? 0 : screen === "complete" ? 2 : 1;
  const selectedCompanion = useMemo(
    () => companions.find((item) => item.id === companion)!,
    [companion],
  );

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(""), 3600);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const openExternal = async (url: string) => {
    if (isTauriRuntime()) {
      await invoke("open_external", { url });
    } else {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  };

  const copyQq = async () => {
    try {
      if (isTauriRuntime()) {
        await invoke("copy_qq");
      } else {
        await navigator.clipboard.writeText(QQ_NUMBER);
      }
      setToast("QQ 号已复制，可以直接粘贴到 QQ 搜索");
    } catch {
      await navigator.clipboard.writeText(QQ_NUMBER).catch(() => undefined);
      setToast("QQ 号已复制");
    }
  };

  const launchInstalledApp = async (target: string, label: string) => {
    try {
      if (!isTauriRuntime()) {
        setToast(`请在桌面版中启动 ${label}`);
        return;
      }
      await invoke("launch_installed_app", { target });
      setToast(`正在启动 ${label}`);
    } catch (caught) {
      setToast(typeof caught === "string" ? caught : `无法启动 ${label}`);
    }
  };

  const runWebPreview = async (runId: number) => {
    const previewEvents: ProgressEvent[] = [
      { runId, percent: 8, phase: "check", title: "正在检查电脑环境", detail: "确认系统版本与安装目录", component: "环境检查" },
      { runId, percent: 28, phase: "download", title: "正在下载 Codex 桌面版", detail: "从 OpenAI 官方应用下载服务获取", component: "Codex" },
      { runId, percent: 54, phase: "install", title: "正在安装 Codex 桌面版", detail: "部署 OpenAI 官方签名的 Windows 应用包", component: "Codex" },
      { runId, percent: 72, phase: "download", title: `正在下载 ${selectedCompanion.name}`, detail: "获取适用于 Windows 的最新版本", component: selectedCompanion.name },
      { runId, percent: 90, phase: "install", title: `正在安装 ${selectedCompanion.name}`, detail: "安装过程即将完成", component: selectedCompanion.name },
      { runId, percent: 100, phase: "done", title: "全部安装完成", detail: "组件已经可以使用", component: "完成" },
    ];

    for (const item of previewEvents) {
      await new Promise((resolve) => window.setTimeout(resolve, 650));
      if (installationAttempt.current !== runId) return;
      setProgress(item);
      recordActivity(item);
    }

    if (installationAttempt.current !== runId) return;
    setInstallResult({
      codexVersion: "预览模式",
      companionName: selectedCompanion.name,
      installDirectory: "本地用户安装目录",
    });
    setScreen("complete");
  };

  const startInstallation = async () => {
    const runId = installationAttempt.current + 1;
    installationAttempt.current = runId;
    setScreen("installing");
    setError("");
    setActivity([]);

    if (!isTauriRuntime()) {
      await runWebPreview(runId);
      return;
    }

    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<ProgressEvent>("install-progress", ({ payload }) => {
        if (payload.runId !== runId || installationAttempt.current !== runId) return;
        setProgress(payload);
        recordActivity(payload);
      });
      const result = await invoke<InstallResult>("install_components", {
        request: { companion, runId },
      });
      if (installationAttempt.current !== runId) return;
      setInstallResult(result);
      setScreen("complete");
    } catch (caught) {
      if (installationAttempt.current !== runId) return;
      setError(typeof caught === "string" ? caught : "安装没有完成，请重试或联系我们。" );
      setScreen("failed");
    } finally {
      unlisten?.();
    }
  };

  const returnToChoice = async () => {
    const runId = installationAttempt.current;
    installationAttempt.current += 1;
    setScreen("choose");
    setActivity([]);
    setProgress((previous) => ({ ...previous, runId: 0, percent: 0 }));
    if (isTauriRuntime()) {
      await invoke("cancel_installation", { runId }).catch(() => undefined);
    }
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand-lockup">
          <div className="brand-mark"><Code2 size={22} strokeWidth={2.2} /></div>
          <div>
            <strong>Codex Setup</strong>
            <span>Windows 安装助手</span>
          </div>
        </div>

        <nav className="step-list" aria-label="安装步骤">
          {steps.map((step, index) => {
            const Icon = step.icon;
            const state = index < currentStep ? "done" : index === currentStep ? "active" : "pending";
            return (
              <div className={`step-item ${state}`} key={step.label}>
                <div className="step-icon">{state === "done" ? <Check size={17} /> : <Icon size={17} />}</div>
                <div>
                  <span>0{index + 1}</span>
                  <strong>{step.label}</strong>
                </div>
              </div>
            );
          })}
        </nav>

        <div className="sidebar-note">
          <ShieldCheck size={18} />
          <p>Codex 桌面版来自 OpenAI 官方，配置工具来自项目 GitHub Release。</p>
        </div>
      </aside>

      <main className="workspace">
        <header className="support-bar">
          <div className="support-contact">
            <Headphones size={17} />
            <span>安装遇到问题？</span>
            <button className="support-copy" onClick={copyQq} title="点击复制 QQ 号">
              <Copy size={15} /> QQ {QQ_NUMBER}
            </button>
          </div>
          <span className="support-assurance"><ShieldCheck size={15} /> 官方上游下载</span>
        </header>

        {screen === "choose" && (
          <section className="content choose-screen">
            <div className="eyebrow"><TerminalSquare size={16} /> Codex 已包含</div>
            <h1>选择你的配置助手</h1>
            <p className="lead">Codex 会自动安装。再选择一个配置工具，推荐初次使用者选择 Codex++。</p>

            {isPreviewMode && (
              <div className="test-mode-note">
                <FlaskConical size={18} />
                <div><strong>当前为流程测试模式</strong><span>只模拟下载和安装进度，不会下载文件，也不会修改电脑设置。</span></div>
              </div>
            )}

            <div className="codex-included">
              <div className="included-icon"><Bot size={24} /></div>
              <div>
                <strong>OpenAI Codex</strong>
                <span>官方桌面应用 · 自动下载并安装</span>
              </div>
              <div className="included-status"><Check size={15} /> 必选</div>
            </div>

            <div className="choice-grid" role="radiogroup" aria-label="配置助手">
              {companions.map((item) => {
                const Icon = item.icon;
                const checked = companion === item.id;
                return (
                  <button
                    className={`choice-card ${checked ? "selected" : ""}`}
                    key={item.id}
                    role="radio"
                    aria-checked={checked}
                    onClick={() => setCompanion(item.id)}
                  >
                    <span className="radio-indicator">{checked && <Check size={14} />}</span>
                    <div className="choice-heading">
                      <div className="choice-icon"><Icon size={21} /></div>
                      <div><strong>{item.name}</strong><span>{item.badge}</span></div>
                    </div>
                    <p className="choice-summary">{item.summary}</p>
                    <p className="choice-detail">{item.detail}</p>
                    <div className="choice-points">
                      {item.points.map((point) => <span key={point}><Check size={13} /> {point}</span>)}
                    </div>
                  </button>
                );
              })}
            </div>

            <div className="tutorial-shortcut">
              <div><BookOpen size={18} /><span>第一次配置 {selectedCompanion.name}？</span></div>
              <button onClick={() => setTutorial(companion)}>查看配置教程 <ArrowRight size={15} /></button>
            </div>

            <div className="screen-footer">
              <div className="privacy-note"><ShieldCheck size={16} /> 默认安装到当前用户目录，无需全程管理员权限</div>
              <button className="primary-button" onClick={startInstallation}>
                开始安装 <ArrowRight size={18} />
              </button>
            </div>
          </section>
        )}

        {screen === "installing" && (
          <section className="content progress-screen">
            <div className="progress-heading">
              <div className="live-icon"><LoaderCircle size={24} /></div>
              <div>
                <div className="eyebrow">正在自动处理</div>
                <h1>{progress.title}</h1>
                <p className="lead">{progress.detail}</p>
              </div>
              <strong className="progress-number">{progress.percent}%</strong>
            </div>

            <div className="progress-track" role="progressbar" aria-valuenow={progress.percent} aria-valuemin={0} aria-valuemax={100}>
              <div className="progress-fill" style={{ width: `${progress.percent}%` }} />
            </div>

            <div className="progress-layout">
              <div className="component-status">
                <StatusRow icon={TerminalSquare} title="OpenAI Codex" state={progress.percent >= 62 ? "done" : "active"} caption={progress.percent >= 62 ? "已安装并完成验证" : "下载、安装与桌面应用验证"} />
                <StatusRow icon={selectedCompanion.icon} title={selectedCompanion.name} state={progress.percent >= 100 ? "done" : progress.percent >= 62 ? "active" : "waiting"} caption={progress.percent >= 62 ? "正在获取并安装最新版本" : "等待 Codex 安装完成"} />
                <StatusRow icon={ShieldCheck} title="最终检查" state={progress.percent >= 100 ? "done" : progress.percent >= 94 ? "active" : "waiting"} caption="确认命令与组件可以正常使用" />
              </div>

              <div className="activity-panel">
                <div className="panel-title"><Gauge size={17} /> 最近进度</div>
                <div className="activity-list">
                  {activity.length === 0 && <span className="muted">正在启动安装任务...</span>}
                  {activity.map((item, index) => (
                    <div className="activity-item" key={`${item.percent}-${index}`}>
                      <span className="activity-dot" />
                      <div><strong>{item.component}</strong><span>{item.detail}</span></div>
                      <time>{item.percent}%</time>
                    </div>
                  ))}
                </div>
              </div>
            </div>

            <div className="progress-footer">
              <button className="secondary-button" onClick={returnToChoice}><ArrowLeft size={17} /> 返回上一步</button>
              <div className="installing-note"><CircleHelp size={16} /> 返回后会停止当前任务，可重新选择 Codex++ 或 CC Switch。</div>
            </div>
          </section>
        )}

        {screen === "failed" && (
          <section className="content result-screen">
            <div className="result-icon error"><XCircle size={30} /></div>
            <div className="eyebrow">安装暂未完成</div>
            <h1>我们在安装时遇到了问题</h1>
            <p className="lead">{error}</p>
            <div className="error-detail">
              <Wrench size={20} />
              <div><strong>需要协助？</strong><span>点击左上角 QQ 联系我们，并把此处错误信息一并发来。</span></div>
            </div>
            <div className="result-actions">
              <button className="secondary-button" onClick={() => setScreen("choose")}><ArrowLeft size={17} /> 返回选择</button>
              <button className="primary-button" onClick={startInstallation}><RefreshCw size={17} /> 重新安装</button>
            </div>
          </section>
        )}

        {screen === "complete" && (
          <section className="content complete-screen">
            <div className="complete-heading">
              <div className="result-icon success"><CheckCircle2 size={30} /></div>
              <div>
                <div className="eyebrow">安装成功</div>
                <h1>Codex 已经准备好了</h1>
                <p className="lead">Codex 与 {installResult?.companionName ?? selectedCompanion.name} 均已完成安装。</p>
              </div>
            </div>

            <div className="completion-meta">
              <span><Check size={15} /> Codex {installResult?.codexVersion ?? "已安装"}</span>
              <span><Check size={15} /> {installResult?.companionName ?? selectedCompanion.name}</span>
              <span><Check size={15} /> {isPreviewMode ? "未修改电脑设置" : "环境变量已配置"}</span>
            </div>

            <div className="launch-band">
              <div className="launch-copy">
                <span className="launch-icon"><Play size={20} /></span>
                <div><strong>现在启动</strong><span>安装流程已经结束，可以直接打开刚才选择的工具。</span></div>
              </div>
              <div className="launch-actions">
                <button className="secondary-button" onClick={() => setTutorial(companion)}>
                  <BookOpen size={17} /> 配置教程
                </button>
                {companion === "codex-plus-plus" ? (
                  <>
                    <button className="secondary-button" onClick={() => launchInstalledApp("codex-plus-plus-manager", "Codex++ 管理工具")}>
                      <Settings2 size={17} /> Codex++ 管理工具
                    </button>
                    <button className="primary-button" onClick={() => launchInstalledApp("codex-plus-plus", "Codex++")}>
                      <Play size={17} /> 启动 Codex++
                    </button>
                  </>
                ) : (
                  <>
                    <button className="primary-button" onClick={() => launchInstalledApp("cc-switch", "CC Switch")}>
                      <Play size={17} /> 启动 CC Switch
                    </button>
                    <button className="secondary-button" onClick={() => launchInstalledApp("codex", "Codex")}>
                      <TerminalSquare size={17} /> 启动 Codex
                    </button>
                  </>
                )}
              </div>
            </div>

            <div className="recommendation-band">
              <div className="recommendation-copy">
                <span className="recommendation-label">接下来推荐</span>
                <h2>用 LLM Free 体验模型服务</h2>
                <p>支持试用，可在开始长期使用前先体验服务。点击预览图或按钮即可访问。</p>
                <button className="primary-button light" onClick={() => openExternal(LLMFREE_URL)}>
                  前往 LLM Free <ExternalLink size={17} />
                </button>
              </div>
              <button className="site-preview" onClick={() => openExternal(LLMFREE_URL)} aria-label="打开 LLM Free 网站">
                <img src={llmfreePreview} alt="LLM Free 网站首页预览" />
                <span><ExternalLink size={15} /> www.llmfree.work</span>
              </button>
            </div>

            <div className="complete-footer">
              <button className="inline-copy" onClick={copyQq}><Copy size={15} /> 使用中有问题，复制 QQ {QQ_NUMBER}</button>
              <button className="secondary-button" onClick={() => openExternal(LLMFREE_URL)}>打开网站 <ExternalLink size={16} /></button>
            </div>
          </section>
        )}
      </main>

      {toast && <div className="toast"><CheckCircle2 size={17} /> {toast}</div>}
      {tutorial && <TutorialModal tool={tutorial} onToolChange={setTutorial} onClose={() => setTutorial(null)} />}
    </div>
  );
}

function TutorialModal({
  tool,
  onToolChange,
  onClose,
}: {
  tool: Companion;
  onToolChange: (tool: Companion) => void;
  onClose: () => void;
}) {
  const content = tutorialContent[tool];
  const [slideIndex, setSlideIndex] = useState(0);
  const [imageViewerOpen, setImageViewerOpen] = useState(false);
  const activeSlideIndex = Math.min(slideIndex, content.slides.length - 1);
  const slide = content.slides[activeSlideIndex];

  useEffect(() => {
    setSlideIndex(0);
  }, [tool]);

  useEffect(() => {
    const handleKeyboard = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (imageViewerOpen) setImageViewerOpen(false);
        else onClose();
        return;
      }
      if (event.key === "ArrowLeft") setSlideIndex((current) => Math.max(0, current - 1));
      if (event.key === "ArrowRight") setSlideIndex((current) => Math.min(content.slides.length - 1, current + 1));
    };
    window.addEventListener("keydown", handleKeyboard);
    return () => window.removeEventListener("keydown", handleKeyboard);
  }, [content.slides.length, imageViewerOpen, onClose]);

  const moveSlide = (direction: -1 | 1) => {
    setSlideIndex((current) => Math.min(content.slides.length - 1, Math.max(0, current + direction)));
  };

  const changeTool = (nextTool: Companion) => {
    setSlideIndex(0);
    onToolChange(nextTool);
  };

  return (
    <>
    <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="tutorial-modal" role="dialog" aria-modal="true" aria-labelledby="tutorial-title">
        <header className="tutorial-header">
          <div><BookOpen size={21} /><div><span>新手配置指南</span><h2 id="tutorial-title">{content.name} 配置教程</h2></div></div>
          <button className="icon-button" onClick={onClose} title="关闭教程"><X size={19} /></button>
        </header>

        <div className="tutorial-tabs" role="tablist" aria-label="选择教程">
          <button role="tab" aria-selected={tool === "codex-plus-plus"} onClick={() => changeTool("codex-plus-plus")}>Codex++</button>
          <button role="tab" aria-selected={tool === "cc-switch"} onClick={() => changeTool("cc-switch")}>CC Switch</button>
        </div>

        <p className="tutorial-intro">{content.intro}</p>

        <div className="tutorial-stage">
          <img
            src={slide.image}
            alt={`${content.name} 第 ${activeSlideIndex + 1} 步截图标注`}
            onClick={() => setImageViewerOpen(true)}
            title="查看大图"
          />
          <button className="tutorial-arrow previous" onClick={() => moveSlide(-1)} disabled={activeSlideIndex === 0} title="上一步" aria-label="上一步">
            <ArrowLeft size={20} />
          </button>
          <button className="tutorial-arrow next" onClick={() => moveSlide(1)} disabled={activeSlideIndex === content.slides.length - 1} title="下一步" aria-label="下一步">
            <ArrowRight size={20} />
          </button>
        </div>

        <div className="tutorial-caption">
          <div><span className="tutorial-number">{activeSlideIndex + 1}</span><div><strong>{slide.title}</strong><p>{slide.body}</p></div></div>
          <span className="tutorial-counter">{activeSlideIndex + 1} / {content.slides.length}</span>
        </div>

        <div className="tutorial-dots" aria-label="教程步骤">
          {content.slides.map((item, index) => (
            <button key={item.title} className={index === activeSlideIndex ? "active" : ""} onClick={() => setSlideIndex(index)} aria-label={`第 ${index + 1} 步`} />
          ))}
        </div>

        <div className="tutorial-note"><KeyRound size={18} /><p><strong>配置提示</strong><span>{content.note}</span></p></div>
        <footer className="tutorial-footer">
          <span><ShieldCheck size={15} /> API 密钥只保存在你自己的电脑和所选工具中</span>
          <button className="primary-button" onClick={onClose}><Check size={17} /> 我知道了</button>
        </footer>
      </section>
    </div>
    {imageViewerOpen && (
      <ImageViewer
        src={slide.image}
        alt={`${content.name}：${slide.title}`}
        canPrevious={activeSlideIndex > 0}
        canNext={activeSlideIndex < content.slides.length - 1}
        onPrevious={() => moveSlide(-1)}
        onNext={() => moveSlide(1)}
        onClose={() => setImageViewerOpen(false)}
      />
    )}
    </>
  );
}

function ImageViewer({
  src,
  alt,
  canPrevious,
  canNext,
  onPrevious,
  onNext,
  onClose,
}: {
  src: string;
  alt: string;
  canPrevious: boolean;
  canNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  onClose: () => void;
}) {
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const dragStart = useRef<{ pointerX: number; pointerY: number; offsetX: number; offsetY: number } | null>(null);

  const resetView = () => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  };

  useEffect(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, [src]);

  const changeScale = (nextScale: number) => {
    const clamped = Math.min(5, Math.max(1, nextScale));
    setScale(clamped);
    if (clamped === 1) setOffset({ x: 0, y: 0 });
  };

  return (
    <div
      className={`image-viewer ${dragStart.current ? "dragging" : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label="教程大图"
      onWheel={(event) => {
        event.preventDefault();
        changeScale(scale + (event.deltaY < 0 ? 0.25 : -0.25));
      }}
      onPointerDown={(event) => {
        if ((event.target as HTMLElement).closest("button")) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        dragStart.current = { pointerX: event.clientX, pointerY: event.clientY, offsetX: offset.x, offsetY: offset.y };
      }}
      onPointerMove={(event) => {
        if (!dragStart.current) return;
        setOffset({
          x: dragStart.current.offsetX + event.clientX - dragStart.current.pointerX,
          y: dragStart.current.offsetY + event.clientY - dragStart.current.pointerY,
        });
      }}
      onPointerUp={() => { dragStart.current = null; }}
      onPointerCancel={() => { dragStart.current = null; }}
      onDoubleClick={resetView}
    >
      <div className="image-viewer-toolbar">
        <button className="viewer-icon-button" onClick={() => changeScale(scale - 0.25)} disabled={scale <= 1} title="缩小"><ZoomOut size={20} /></button>
        <button className="viewer-icon-button" onClick={() => changeScale(scale + 0.25)} disabled={scale >= 5} title="放大"><ZoomIn size={20} /></button>
        <button className="viewer-icon-button" onClick={resetView} title="复位"><Maximize2 size={20} /></button>
        <button className="viewer-icon-button" onClick={onClose} title="关闭大图"><X size={21} /></button>
      </div>
      <button
        className="image-viewer-nav previous"
        onClick={onPrevious}
        disabled={!canPrevious}
        title="上一张"
        aria-label="上一张"
      ><ArrowLeft size={25} /></button>
      <button
        className="image-viewer-nav next"
        onClick={onNext}
        disabled={!canNext}
        title="下一张"
        aria-label="下一张"
      ><ArrowRight size={25} /></button>
      <img
        src={src}
        alt={alt}
        draggable={false}
        style={{ transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})` }}
      />
    </div>
  );
}

function StatusRow({
  icon: Icon,
  title,
  caption,
  state,
}: {
  icon: typeof Bot;
  title: string;
  caption: string;
  state: "waiting" | "active" | "done";
}) {
  return (
    <div className={`status-row ${state}`}>
      <div className="status-row-icon"><Icon size={20} /></div>
      <div><strong>{title}</strong><span>{caption}</span></div>
      <div className="status-symbol">
        {state === "done" ? <Check size={16} /> : state === "active" ? <LoaderCircle size={17} /> : <span />}
      </div>
    </div>
  );
}

export default App;
