import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Snapshot = {
  status: string;
  error: string;
  settings_json: string;
};

type HotkeyDoc = {
  id: string;
  shortcut: string;
  action: string;
  enabled: boolean;
};

type SettingsDoc = {
  schema_version: number;
  microphone: { target: string; mute_on_start: boolean };
  hotkeys: HotkeyDoc[];
  general: {
    start_minimized: boolean;
    close_to_tray: boolean;
    start_with_system: boolean;
  };
  toast: { enabled: boolean; position: string; sound: string };
};

const ACTIONS = [
  ["toggle_mute", "Toggle mute"],
  ["force_mute", "Mute"],
  ["force_unmute", "Unmute"],
  ["hold_to_mute", "Hold to mute"],
  ["open_settings", "Open settings"],
] as const;

const TOAST_PLACES = [
  ["top-left", "Top left"],
  ["top", "Top"],
  ["top-right", "Top right"],
  ["left", "Left"],
  ["center", "Center"],
  ["right", "Right"],
  ["bottom-left", "Bottom left"],
  ["bottom", "Bottom"],
  ["bottom-right", "Bottom right"],
] as const;

const EMPTY_DOC: SettingsDoc = {
  schema_version: 1,
  microphone: { target: "default", mute_on_start: false },
  hotkeys: [{ id: "toggle-default-mic", shortcut: "F9", action: "toggle_mute", enabled: true }],
  general: { start_minimized: true, close_to_tray: true, start_with_system: false },
  toast: { enabled: true, position: "bottom", sound: "" },
};

const KEY_NAMES: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Minus: "Minus",
  Equal: "Equal",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Backquote: "Backquote",
};

function readSettings(settingsJson: string): SettingsDoc {
  const value = JSON.parse(settingsJson) as {
    schema_version?: number;
    microphone?: { target?: string; mute_on_start?: boolean };
    hotkeys?: { id?: string; shortcut?: string; action?: string; enabled?: boolean }[];
    general?: {
      start_minimized?: boolean;
      close_to_tray?: boolean;
      start_with_system?: boolean;
    };
    toast?: { enabled?: boolean; position?: string; sound?: string };
  };
  return {
    schema_version: value.schema_version ?? 1,
    microphone: {
      target: value.microphone?.target || "default",
      mute_on_start: value.microphone?.mute_on_start === true,
    },
    hotkeys: (value.hotkeys ?? []).map((item, index) => ({
      id: item.id || `hotkey-${index}`,
      shortcut: item.shortcut ?? "",
      action: item.action || "toggle_mute",
      enabled: item.enabled !== false,
    })),
    general: {
      start_minimized: value.general?.start_minimized === true,
      close_to_tray: value.general?.close_to_tray !== false,
      start_with_system: value.general?.start_with_system === true,
    },
    toast: {
      enabled: value.toast?.enabled !== false,
      position: value.toast?.position || "bottom",
      sound: value.toast?.sound ?? "",
    },
  };
}

function shortcutFromEvent(event: KeyboardEvent): string | null {
  if (event.repeat || event.key === "Escape") return null;
  let key: string | null = null;
  if (event.code.startsWith("Key") && event.code.length === 4) key = event.code.slice(3);
  else if (event.code.startsWith("Digit")) key = event.code.slice(5);
  else if (/^F\d{1,2}$/.test(event.code)) key = event.code;
  else key = KEY_NAMES[event.code] ?? null;
  if (!key) return null;
  const parts = [
    event.ctrlKey ? "Ctrl" : "",
    event.altKey ? "Alt" : "",
    event.shiftKey ? "Shift" : "",
    event.metaKey ? "Super" : "",
    key,
  ].filter(Boolean);
  return parts.join("+");
}

function statusCopy(status: string) {
  if (status === "Muted") return { title: "Microphone is muted", button: "Unmute microphone" };
  if (status === "Unmuted") return { title: "Microphone is active", button: "Mute microphone" };
  return { title: "Microphone is unavailable", button: "Mute microphone" };
}

function App() {
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [doc, setDoc] = useState<SettingsDoc>(EMPTY_DOC);
  const [recordingId, setRecordingId] = useState<string | null>(null);
  const [nav, setNav] = useState<"mic" | "keys" | "settings">("mic");
  const [keysOpen, setKeysOpen] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(true);
  const saveSeq = useRef(0);
  const keysRef = useRef<HTMLElement>(null);
  const settingsRef = useRef<HTMLElement>(null);

  useEffect(() => {
    let cancelled = false;
    const stops: Array<() => void> = [];

    invoke<Snapshot>("snapshot")
      .then((shot) => {
        if (cancelled) return;
        setStatus(shot.status);
        setError(shot.error);
        setSaved(shot.error === "");
        try {
          setDoc(readSettings(shot.settings_json));
        } catch (parseError) {
          setError(String(parseError));
          setSaved(false);
        }
      })
      .catch((invokeError: unknown) => {
        if (!cancelled) setError(String(invokeError));
      });

    listen<string>("mic-status", (event) => {
      setStatus(event.payload);
    }).then((stop) => {
      if (cancelled) stop();
      else stops.push(stop);
    });

    listen<string>("hotkey-error", (event) => {
      setError(event.payload);
      setSaved(event.payload === "");
    }).then((stop) => {
      if (cancelled) stop();
      else stops.push(stop);
    });

    return () => {
      cancelled = true;
      for (const stop of stops) stop();
    };
  }, []);

  useEffect(() => {
    if (!recordingId) return;
    function onKey(event: KeyboardEvent) {
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "Escape") {
        setRecordingId(null);
        return;
      }
      const shortcut = shortcutFromEvent(event);
      if (!shortcut) return;
      const hotkeys = doc.hotkeys.map((hotkey) =>
        hotkey.id === recordingId ? { ...hotkey, shortcut } : hotkey,
      );
      setRecordingId(null);
      void persist({ ...doc, hotkeys });
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recordingId, doc]);

  async function persist(next: SettingsDoc) {
    const seq = ++saveSeq.current;
    setDoc(next);
    try {
      const shot = await invoke<Snapshot>("save_settings", {
        settingsJson: JSON.stringify(next),
      });
      if (seq !== saveSeq.current) return;
      setStatus(shot.status);
      setError(shot.error);
      setSaved(shot.error === "");
      try {
        setDoc(readSettings(shot.settings_json));
      } catch (parseError) {
        setError(String(parseError));
        setSaved(false);
      }
    } catch (saveError) {
      if (seq !== saveSeq.current) return;
      setError(String(saveError));
      setSaved(false);
    }
  }

  function show(section: "mic" | "keys" | "settings") {
    setNav(section);
    const target = section === "keys" ? keysRef.current : section === "settings" ? settingsRef.current : null;
    target?.scrollIntoView({ block: "start" });
    if (section === "keys") setKeysOpen(true);
    if (section === "settings") setSettingsOpen(true);
  }

  const copy = statusCopy(status);
  const holds = doc.hotkeys.some((hotkey) => hotkey.enabled && hotkey.action === "hold_to_mute");

  return (
    <div className="app">
      <aside className="sidebar">
        <img className="brand" src="/logo.png" alt="MICRIX" />
        <nav className="nav" aria-label="Sections">
          <button type="button" className={nav === "mic" ? "nav-item active" : "nav-item"} onClick={() => show("mic")}>
            <MicIcon />
            Microphone
          </button>
          <button type="button" className={nav === "keys" ? "nav-item active" : "nav-item"} onClick={() => show("keys")}>
            <KeysIcon />
            Shortcuts
          </button>
          <button
            type="button"
            className={nav === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => show("settings")}
          >
            <GearIcon />
            Settings
          </button>
        </nav>
        <p className="tray-state">
          <span className="dot" aria-hidden="true" />
          Running in tray
        </p>
      </aside>

      <main className="main">
        <header className="page-head">
          <h1>Microphone</h1>
          <p>Manage your input device</p>
        </header>

        <section className="card status-card" aria-live="polite">
          <span className="mic-badge" data-state={status}>
            <MicIcon />
          </span>
          <div className="status-copy">
            <strong data-state={status}>{copy.title}</strong>
            <span>Default input device</span>
          </div>
          <button
            type="button"
            className="mute"
            disabled={status !== "Muted" && status !== "Unmuted"}
            onClick={() => {
              invoke("toggle_microphone").catch((toggleError: unknown) => {
                setError(String(toggleError));
                setSaved(false);
              });
            }}
          >
            <MicIcon />
            {copy.button}
          </button>
        </section>

        <section className="card" ref={keysRef} id="shortcuts">
          <button type="button" className="card-head" onClick={() => setKeysOpen((open) => !open)} aria-expanded={keysOpen}>
            <KeysIcon />
            <span>
              <strong>Keyboard shortcuts</strong>
              <small>Quick actions for your microphone</small>
            </span>
            <Chevron open={keysOpen} />
          </button>
          {keysOpen ? (
            <div className="card-body">
              {doc.hotkeys.filter((hotkey) => hotkey.enabled).map((hotkey) => (
                <div className="shortcut" key={hotkey.id}>
                  <button
                    type="button"
                    className={recordingId === hotkey.id ? "key recording" : "key"}
                    onClick={() => setRecordingId(hotkey.id)}
                  >
                    {recordingId === hotkey.id ? "Press keys" : hotkey.shortcut || "Record"}
                  </button>
                  <select
                    aria-label="Action"
                    value={hotkey.action}
                    onChange={(event) => {
                      const hotkeys = doc.hotkeys.map((item) =>
                        item.id === hotkey.id ? { ...item, action: event.currentTarget.value } : item,
                      );
                      void persist({ ...doc, hotkeys });
                    }}
                  >
                    {ACTIONS.map(([value, label]) => (
                      <option key={value} value={value}>
                        {label}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label="Record shortcut"
                    onClick={() => setRecordingId(hotkey.id)}
                  >
                    <PencilIcon />
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label="Remove shortcut"
                    onClick={() =>
                      void persist({
                        ...doc,
                        hotkeys: doc.hotkeys.filter((item) => item.id !== hotkey.id),
                      })
                    }
                  >
                    <TrashIcon />
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="add"
                onClick={() => {
                  const id = crypto.randomUUID();
                  setDoc({
                    ...doc,
                    hotkeys: [...doc.hotkeys, { id, shortcut: "", action: "toggle_mute", enabled: true }],
                  });
                  setRecordingId(id);
                  setKeysOpen(true);
                }}
              >
                + Add shortcut
              </button>
              {holds ? (
                <p className="hint">Hold to mute puts the microphone back when the keys are released.</p>
              ) : null}
            </div>
          ) : null}
        </section>

        <section className="card" ref={settingsRef} id="settings">
          <button
            type="button"
            className="card-head"
            onClick={() => setSettingsOpen((open) => !open)}
            aria-expanded={settingsOpen}
          >
            <GearIcon />
            <span>
              <strong>Startup & behavior</strong>
              <small>Control how MICRIX works</small>
            </span>
            <Chevron open={settingsOpen} />
          </button>
          {settingsOpen ? (
            <div className="card-body settings">
              <Toggle
                label="Mute when MICRIX starts"
                checked={doc.microphone.mute_on_start}
                title="Applies the next time MICRIX starts."
                onChange={(muteOnStart) =>
                  void persist({ ...doc, microphone: { ...doc.microphone, mute_on_start: muteOnStart } })
                }
              />
              <Toggle
                label="Start in system tray"
                checked={doc.general.start_minimized}
                onChange={(startMinimized) =>
                  void persist({ ...doc, general: { ...doc.general, start_minimized: startMinimized } })
                }
              />
              <Toggle
                label="Hide on close"
                checked={doc.general.close_to_tray}
                onChange={(closeToTray) =>
                  void persist({ ...doc, general: { ...doc.general, close_to_tray: closeToTray } })
                }
              />
              <Toggle
                label="Launch on Windows startup"
                checked={doc.general.start_with_system}
                onChange={(startWithSystem) =>
                  void persist({ ...doc, general: { ...doc.general, start_with_system: startWithSystem } })
                }
              />
              <Toggle
                label="Show mute toast"
                checked={doc.toast.enabled}
                title="Shows a message when the microphone is muted or unmuted."
                onChange={(toastEnabled) =>
                  void persist({ ...doc, toast: { ...doc.toast, enabled: toastEnabled } })
                }
              />
              <p className="hint">Where it shows</p>
              <div className="layout-pad" role="group" aria-label="Toast position">
                {TOAST_PLACES.map(([place, label]) => (
                  <button
                    key={place}
                    type="button"
                    className={doc.toast.position === place ? "active" : undefined}
                    aria-label={label}
                    aria-pressed={doc.toast.position === place}
                    onClick={() => void persist({ ...doc, toast: { ...doc.toast, position: place } })}
                  />
                ))}
              </div>
              <div className="sound-row">
                <span>{doc.toast.sound ? doc.toast.sound.split(/[/\\]/).pop() : "No sound"}</span>
                <button
                  type="button"
                  className="add"
                  onClick={() => {
                    invoke<string>("pick_sound")
                      .then((picked) => {
                        if (picked) void persist({ ...doc, toast: { ...doc.toast, sound: picked } });
                      })
                      .catch((pickError: unknown) => {
                        setError(String(pickError));
                        setSaved(false);
                      });
                  }}
                >
                  Add sound
                </button>
                {doc.toast.sound ? (
                  <button
                    type="button"
                    className="add"
                    onClick={() => void persist({ ...doc, toast: { ...doc.toast, sound: "" } })}
                  >
                    Remove
                  </button>
                ) : null}
              </div>
              <p className="hint">WAV file. Plays on mute and unmute.</p>
            </div>
          ) : null}
        </section>

        <footer className="page-foot">
          <p className={error ? "save-state bad" : "save-state"} role={error ? "alert" : undefined}>
            <CheckIcon />
            {error || (saved ? "Changes saved" : "Saving…")}
          </p>
          <span className="version">v0.1.4</span>
        </footer>
      </main>
    </div>
  );
}

function Toggle({
  label,
  checked,
  title,
  onChange,
}: {
  label: string;
  checked: boolean;
  title?: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="toggle" title={title}>
      <span>{label}</span>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
    </label>
  );
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg className={open ? "chevron open" : "chevron"} viewBox="0 0 20 20" aria-hidden="true">
      <path d="M5 8l5 5 5-5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function MicIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M12 14a3 3 0 0 0 3-3V7a3 3 0 0 0-6 0v4a3 3 0 0 0 3 3z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path d="M6.5 11a5.5 5.5 0 0 0 11 0M12 16.5V20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function KeysIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <rect x="3" y="6" width="18" height="12" rx="2" fill="none" stroke="currentColor" strokeWidth="1.6" />
      <path d="M7 10h.01M11 10h.01M15 10h.01M8 14h8" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" strokeWidth="1.6" />
      <path
        d="M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4 18 18M18 6l-1.6 1.6M7.6 16.4 6 18"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function PencilIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 20h4l10-10-4-4L4 16v4z" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
      <path d="M13 7l4 4" fill="none" stroke="currentColor" strokeWidth="1.6" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M5 7h14M9 7V5h6v2M8 7l1 13h6l1-13" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" strokeWidth="1.6" />
      <path d="M8.5 12.2l2.2 2.2 4.8-5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

export default App;
