import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";

type ContentType = "post_content" | "boosty_videos" | "external_videos" | "files" | "audio";
type Settings = {
  username: string; authHeader: string; cookie: string; destinationDirectory: string;
  postUrl: string; preferredVideoQuality: string; contentTypes: ContentType[];
  requestDelaySeconds: number; skipAllFailures: boolean;
};
type RuntimeStatus = { installed: boolean; version: string | null; running: boolean };
type DownloadEvent = {
  kind: "started" | "log" | "error" | "completed" | "failed" | "cancelled" | "progress";
  message: string;
  percent?: number | null;
  label?: string;
  detail?: string;
  active?: boolean;
};
type ProgressState = { label: string; percent: number | null; detail: string };
type LogLine = DownloadEvent & { id: number };

const contentOptions: Array<{ value: ContentType; label: string; hint: string }> = [
  { value: "post_content", label: "Посты и фото", hint: "HTML-копии публикаций" },
  { value: "boosty_videos", label: "Видео Boosty", hint: "Встроенные видео" },
  { value: "external_videos", label: "Внешние видео", hint: "YouTube и Vimeo" },
  { value: "files", label: "Файлы", hint: "Прикреплённые материалы" },
  { value: "audio", label: "Аудио", hint: "Аудиозаписи из постов" },
];

const authHelperScript = `(function () {
  const getToken = () => {
    const raw = document.cookie.split("; ").find((item) => item.startsWith("auth="))?.split("=").slice(1).join("=");
    if (!raw) return null;
    try { return JSON.parse(decodeURIComponent(raw)).accessToken || null; } catch { return null; }
  };
  const show = () => {
    const token = getToken(); if (!token) return;
    document.getElementById("boosty-loader-credentials")?.remove();
    const box = document.createElement("div"); box.id = "boosty-loader-credentials";
    box.style.cssText = "position:fixed;top:20px;right:20px;width:440px;max-height:80vh;overflow:auto;padding:18px;background:#fff;color:#222;border-radius:14px;z-index:2147483647;box-shadow:0 12px 50px #0005;font:14px system-ui";
    const field = (title, value) => {
      const wrap = document.createElement("div"); wrap.style.marginBottom = "14px";
      const heading = document.createElement("b"); heading.textContent = title;
      const code = document.createElement("code"); code.textContent = value; code.style.cssText = "display:block;margin:7px 0;padding:10px;background:#f2f2f2;border-radius:8px;word-break:break-all";
      const button = document.createElement("button"); button.textContent = "Скопировать"; button.onclick = () => navigator.clipboard.writeText(value);
      wrap.append(heading, code, button); return wrap;
    };
    box.append(field("Authorization", "Bearer " + token), field("Cookie", document.cookie));
    const close = document.createElement("button"); close.textContent = "Закрыть"; close.onclick = () => box.remove(); box.append(close); document.body.append(box);
  };
  show(); const originalFetch = window.fetch; window.fetch = async (...args) => { show(); return originalFetch.apply(window, args); };
  console.log("Boosty Loader: данные появятся в правом верхнем углу. Прокрутите страницу, если блок ещё не виден.");
})();`;

const emptyStatus: RuntimeStatus = { installed: false, version: null, running: false };
const ansiPattern = /[\u001B\u009B][[\]()#;?]*(?:(?:(?:[a-zA-Z\d]*(?:;[-a-zA-Z\d\/#&.:=?%@~_]+)*)?\u0007)|(?:(?:\d{1,4}(?:[;:]\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~]))/g;

function Icon({ name }: { name: "download" | "folder" | "key" | "terminal" | "check" | "stop" | "copy" | "eye" }) {
  const paths: Record<typeof name, React.ReactNode> = {
    download: <><path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M5 20h14"/></>,
    folder: <path d="M3 7.5a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>,
    key: <><circle cx="8" cy="15" r="4"/><path d="m11 12 8-8m-3 3 2 2m-5 1 2 2"/></>,
    terminal: <><path d="m5 7 4 4-4 4"/><path d="M12 16h7"/></>,
    check: <path d="m5 12 4 4L19 6"/>, stop: <rect x="6" y="6" width="12" height="12" rx="2"/>,
    copy: <><rect x="8" y="8" width="11" height="11" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></>,
    eye: <><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z"/><circle cx="12" cy="12" r="2.5"/></>,
  };
  return <svg aria-hidden="true" viewBox="0 0 24 24">{paths[name]}</svg>;
}

function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [runtime, setRuntime] = useState<RuntimeStatus>(emptyStatus);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [progress, setProgress] = useState<ProgressState | null>(null);
  const [notice, setNotice] = useState(""); const [error, setError] = useState("");
  const [busy, setBusy] = useState(false); const [showSecrets, setShowSecrets] = useState(false); const [showAdvanced, setShowAdvanced] = useState(false);
  const logId = useRef(0); const logEnd = useRef<HTMLDivElement>(null);

  const refreshStatus = async () => setRuntime(await invoke<RuntimeStatus>("runtime_status"));
  const installAttempted = useRef(false);

  const install = async () => {
    setBusy(true); setError(""); setNotice("Устанавливаю Boosty Downloader…");
    try {
      await invoke<string>("install_downloader");
      await refreshStatus();
      setNotice("Boosty Downloader готов к работе");
    } catch (reason) {
      setError(String(reason));
      setNotice("");
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void Promise.all([invoke<Settings>("load_settings"), invoke<RuntimeStatus>("runtime_status")])
      .then(([saved, status]) => {
        setSettings(saved);
        setRuntime(status);
        if (!status.installed && !installAttempted.current) {
          installAttempted.current = true;
          void install();
        }
      })
      .catch((reason) => setError(String(reason)));
    const unsubscribe = listen<DownloadEvent>("download-event", ({ payload }) => {
      const cleaned = { ...payload, message: payload.message.replace(ansiPattern, "") };
      if (cleaned.kind === "progress") {
        if (cleaned.active === false) setProgress(null);
        else setProgress({ label: cleaned.label?.trim() || cleaned.message || "Загрузка", percent: cleaned.percent ?? null, detail: cleaned.detail?.trim() || "" });
        return;
      }
      setLogs((current) => [...current.slice(-499), { ...cleaned, id: ++logId.current }]);
      if (["completed", "failed", "cancelled"].includes(cleaned.kind)) setProgress(null);
      if (["started", "completed", "failed", "cancelled"].includes(cleaned.kind)) void refreshStatus();
    });
    return () => { void unsubscribe.then((fn) => fn()); };
  }, []);
  useEffect(() => { logEnd.current?.scrollIntoView({ behavior: "smooth" }); }, [logs]);

  const canStart = useMemo(() => Boolean(settings?.username.trim() && settings.authHeader.trim() && settings.cookie.trim() && settings.destinationDirectory.trim() && settings.contentTypes.length && runtime.installed && !runtime.running), [settings, runtime]);
  const patchSettings = <K extends keyof Settings>(key: K, value: Settings[K]) => { setSettings((current) => current ? { ...current, [key]: value } : current); setNotice(""); setError(""); };
  const toggleContent = (value: ContentType) => { if (settings) patchSettings("contentTypes", settings.contentTypes.includes(value) ? settings.contentTypes.filter((item) => item !== value) : [...settings.contentTypes, value]); };

  const copyHelper = async () => { try { await navigator.clipboard.writeText(authHelperScript); setNotice("Helper-скрипт скопирован. Откройте Boosty и вставьте его в консоль браузера."); } catch { setError("Не удалось скопировать скрипт в буфер обмена."); } };
  const chooseDestination = async () => { const selected = await open({ directory: true, multiple: false, title: "Папка для загрузок" }); if (typeof selected === "string") patchSettings("destinationDirectory", selected); };
  const openDestination = async () => {
    const path = settings?.destinationDirectory.trim();
    if (!path) { setError("Сначала укажите папку для загрузок."); return; }
    try { await openPath(path); } catch (reason) { setError(`Не удалось открыть папку: ${String(reason)}`); }
  };
  const save = async () => { if (!settings) return; setBusy(true); setError(""); try { await invoke("save_settings", { settings }); setNotice("Настройки сохранены локально"); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const start = async () => { if (!settings) return; setBusy(true); setError(""); setNotice(""); setLogs([]); setProgress(null); try { await invoke("start_download", { settings }); setRuntime((current) => ({ ...current, running: true })); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } };
  const stop = async () => { try { await invoke("stop_download"); } catch (reason) { setError(String(reason)); } };

  if (!settings) return <main className="loading"><span className="spinner" />Загружаем настройки…</main>;

  return <main className="app-shell">
    <header className="topbar"><div className="brand"><div className="brand-mark"><img src="/app-icon.png" alt="" width={42} height={42} /></div><div><strong>Boosty Loader</strong><span>Сохраняйте доступный вам контент</span></div></div><div className={`runtime-pill ${runtime.installed ? "ready" : "missing"}`}><span className="status-dot" />{runtime.installed ? `Downloader ${runtime.version ?? "готов"}` : busy ? "Установка…" : "Downloader не установлен"}{!runtime.installed && <button className="text-button" onClick={install} disabled={busy}>{busy ? "Подождите" : "Установить"}</button>}</div></header>
    <div className="workspace"><section className="main-column">
      <div className="intro"><span className="eyebrow">Новая загрузка</span><h1>Загрузите материалы автора</h1><p>Приложение синхронизирует новые публикации и пропустит то, что уже было сохранено.</p></div>
      {(notice || error) && <div className={`notice ${error ? "notice-error" : "notice-success"}`}><Icon name={error ? "stop" : "check"} /><span>{error || notice}</span><button aria-label="Закрыть" onClick={() => { setNotice(""); setError(""); }}>×</button></div>}
      <div className="card creator-card"><div className="section-heading"><span className="step">1</span><div><h2>Автор и место сохранения</h2><p>Имя автора — это часть ссылки после boosty.to/</p></div></div><div className="field-grid"><label className="field"><span>Имя автора</span><div className="prefix-input"><b>boosty.to/</b><input value={settings.username} placeholder="creator" onChange={(e) => patchSettings("username", e.target.value.replace(/^https?:\/\/(?:www\.)?boosty\.to\//, "").split("/")[0])} /></div></label><label className="field"><span>Папка для загрузок</span><div className="input-action"><input value={settings.destinationDirectory} onChange={(e) => patchSettings("destinationDirectory", e.target.value)} /><button title="Выбрать папку" onClick={chooseDestination}><Icon name="folder" /></button></div></label></div><label className="field optional"><span>Ссылка на один пост <small>необязательно</small></span><input value={settings.postUrl} placeholder="https://boosty.to/creator/posts/…" onChange={(e) => patchSettings("postUrl", e.target.value)} /></label></div>
      <div className="card auth-card"><div className="section-heading"><span className="step">2</span><div><h2>Доступ к вашему контенту</h2><p>Данные нужны для платных и приватных публикаций</p></div><button className="secondary helper-button" onClick={copyHelper}><Icon name="copy" />Скопировать helper</button></div><div className="helper-note"><Icon name="key" /><div><b>Самый быстрый способ</b><span>Скопируйте helper, войдите на <button onClick={() => openUrl("https://boosty.to")}>boosty.to</button>, откройте консоль браузера (F12) и вставьте скрипт. Он покажет оба значения.</span></div></div><div className="secret-fields"><label className="field"><span>Authorization</span><div className="input-action"><input type={showSecrets ? "text" : "password"} value={settings.authHeader} placeholder="Bearer …" onChange={(e) => patchSettings("authHeader", e.target.value)} /><button title="Показать или скрыть" onClick={() => setShowSecrets((value) => !value)}><Icon name="eye" /></button></div></label><label className="field"><span>Cookie</span><textarea rows={3} value={settings.cookie} placeholder="auth=…; _ga=…" onChange={(e) => patchSettings("cookie", e.target.value)} className={showSecrets ? "" : "masked"} /></label></div><p className="privacy"><span>●</span> Данные хранятся локально на этом устройстве и передаются только CLI-клиенту Boosty Downloader.</p></div>
      <div className="card content-card"><div className="section-heading"><span className="step">3</span><div><h2>Что загрузить</h2><p>Для полной офлайн-копии оставьте всё выбранным</p></div></div><div className="content-options">{contentOptions.map((option) => { const checked = settings.contentTypes.includes(option.value); return <button key={option.value} className={`content-option ${checked ? "selected" : ""}`} onClick={() => toggleContent(option.value)} aria-pressed={checked}><span className="checkbox">{checked && <Icon name="check" />}</span><span><b>{option.label}</b><small>{option.hint}</small></span></button>; })}</div><button className="advanced-toggle" onClick={() => setShowAdvanced((value) => !value)}>{showAdvanced ? "Скрыть" : "Показать"} дополнительные настройки <span>{showAdvanced ? "−" : "+"}</span></button>{showAdvanced && <div className="advanced-panel"><label className="field"><span>Качество видео</span><select value={settings.preferredVideoQuality} onChange={(e) => patchSettings("preferredVideoQuality", e.target.value)}><option value="smallest_size">Минимальный размер</option><option value="low">Низкое</option><option value="medium">Среднее</option><option value="high">Высокое</option><option value="highest">Максимальное</option></select></label><label className="field"><span>Пауза между запросами</span><div className="suffix-input"><input type="number" min="1" step="0.5" value={settings.requestDelaySeconds} onChange={(e) => patchSettings("requestDelaySeconds", Number(e.target.value))} /><b>сек.</b></div></label><label className="switch-row"><input type="checkbox" checked={settings.skipAllFailures} onChange={(e) => patchSettings("skipAllFailures", e.target.checked)} /><span className="switch" /><span><b>Не останавливаться после ошибок</b><small>Пропускать любые неудачные посты</small></span></label></div>}</div>
    </section><aside className="side-column"><div className={`action-card ${runtime.running ? "is-running" : ""}`}><div className="action-icon"><Icon name={runtime.running ? "terminal" : "download"} /></div><h2>{runtime.running ? "Идёт загрузка" : "Всё готово?"}</h2><p>{runtime.running ? (progress ? progress.label : "Окно можно оставить открытым — прогресс появится ниже.") : "Проверьте автора, доступ и выбранные материалы."}</p>{runtime.running && <div className="download-progress" aria-live="polite"><div className={`download-progress-track ${progress?.percent == null ? "is-indeterminate" : ""}`}><div className="download-progress-fill" style={progress?.percent == null ? undefined : { width: `${Math.max(0, Math.min(100, progress.percent))}%` }} /></div><div className="download-progress-meta"><span>{progress?.percent == null ? "…" : `${Math.round(progress.percent)}%`}</span><span>{progress?.detail || "Ожидание данных"}</span></div></div>}{runtime.running ? <button className="danger primary-action" onClick={stop}><Icon name="stop" />Остановить</button> : <button className="primary primary-action" onClick={start} disabled={!canStart || busy}><Icon name="download" />Начать загрузку</button>}<button className="save-button" onClick={save} disabled={busy}>Сохранить настройки</button><div className="action-links"><button onClick={openDestination}><Icon name="folder" />Открыть папку</button><button onClick={() => openUrl("https://github.com/Glitchy-Sheep/boosty-downloader")}><span>↗</span>Документация</button></div></div><div className="log-card"><div className="log-header"><div><Icon name="terminal" /><b>Журнал</b></div>{logs.length > 0 && <button onClick={() => setLogs([])}>Очистить</button>}</div><div className="log-body">{logs.length === 0 ? <div className="log-empty"><span>›_</span><p>Здесь появится ход загрузки</p></div> : logs.map((line) => <div key={line.id} className={`log-line ${line.kind}`}><span>{line.kind === "error" || line.kind === "failed" ? "!" : "›"}</span><p>{line.message}</p></div>)}<div ref={logEnd} /></div></div></aside></div>
    <footer>Boosty Loader использует открытый проект <button onClick={() => openUrl("https://github.com/Glitchy-Sheep/boosty-downloader")}>boosty-downloader</button> · Скачивайте только материалы, к которым у вас есть законный доступ.</footer>
  </main>;
}

export default App;
