import { useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Snapshot = {
  status: string;
  error: string;
  settings_json: string;
};

type HotkeyDraft = {
  id: string;
  shortcut: string;
  action: string;
};

type Draft = {
  hotkeys: HotkeyDraft[];
  muteOnStart: boolean;
  startMinimized: boolean;
  closeToTray: boolean;
  startWithSystem: boolean;
};

const ACTIONS = [
  ["toggle_mute", "Toggle"],
  ["force_mute", "Mute"],
  ["force_unmute", "Unmute"],
  ["hold_to_mute", "Hold to mute"],
  ["open_settings", "Open settings"],
] as const;

const EMPTY_DRAFT: Draft = {
  hotkeys: [{ id: "toggle-default-mic", shortcut: "F9", action: "toggle_mute" }],
  muteOnStart: false,
  startMinimized: true,
  closeToTray: true,
  startWithSystem: false,
};

function readDraft(settingsJson: string): Draft {
  const value = JSON.parse(settingsJson) as {
    microphone?: { mute_on_start?: boolean };
    hotkeys?: { id?: string; shortcut?: string; action?: string; enabled?: boolean }[];
    general?: {
      start_minimized?: boolean;
      close_to_tray?: boolean;
      start_with_system?: boolean;
    };
  };
  const hotkeys = (value.hotkeys ?? [])
    .filter((item) => item.enabled !== false)
    .map((item, index) => ({
      id: item.id || `hotkey-${index}`,
      shortcut: item.shortcut ?? "",
      action: item.action || "toggle_mute",
    }));
  return {
    hotkeys,
    muteOnStart: value.microphone?.mute_on_start === true,
    startMinimized: value.general?.start_minimized === true,
    closeToTray: value.general?.close_to_tray !== false,
    startWithSystem: value.general?.start_with_system === true,
  };
}

function actionLabel(status: string) {
  if (status === "Unmuted") return "Mute";
  if (status === "Muted") return "Unmute";
  return "Toggle microphone";
}

function App() {
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);

  useEffect(() => {
    let cancelled = false;
    const stops: Array<() => void> = [];

    invoke<Snapshot>("snapshot")
      .then((shot) => {
        if (cancelled) return;
        setStatus(shot.status);
        setError(shot.error);
        try {
          setDraft(readDraft(shot.settings_json));
        } catch (parseError) {
          setError(String(parseError));
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
    }).then((stop) => {
      if (cancelled) stop();
      else stops.push(stop);
    });

    return () => {
      cancelled = true;
      for (const stop of stops) stop();
    };
  }, []);

  async function save(event: FormEvent) {
    event.preventDefault();
    const payload = {
      schema_version: 1,
      microphone: { target: "default", mute_on_start: draft.muteOnStart },
      hotkeys: draft.hotkeys.map((hotkey, index) => ({
        id: hotkey.id || `hotkey-${index}`,
        shortcut: hotkey.shortcut.trim(),
        action: hotkey.action,
        enabled: true,
      })),
      general: {
        start_minimized: draft.startMinimized,
        close_to_tray: draft.closeToTray,
        start_with_system: draft.startWithSystem,
      },
    };
    try {
      const shot = await invoke<Snapshot>("save_settings", {
        settingsJson: JSON.stringify(payload),
      });
      setStatus(shot.status);
      setError(shot.error);
      setDraft(readDraft(shot.settings_json));
    } catch (saveError) {
      setError(String(saveError));
    }
  }

  return (
    <main className="sheet">
      <img className="mark" src="/logo.png" alt="MICRIX" />
      <p className="status" data-state={status}>
        {status || "…"}
      </p>
      <p className="target">Default input</p>
      <button
        type="button"
        className="mute"
        data-quiet={status === "Muted" ? "true" : "false"}
        onClick={() => {
          invoke("toggle_microphone").catch((toggleError: unknown) => {
            setError(String(toggleError));
          });
        }}
      >
        {actionLabel(status)}
      </button>
      {error ? (
        <p className="notice" role="alert">
          {error}
        </p>
      ) : null}

      <form onSubmit={save}>
        <div className="hotkeys">
          {draft.hotkeys.map((hotkey, index) => (
            <div className="hotkey" key={hotkey.id || index}>
              <input
                aria-label="Shortcut"
                value={hotkey.shortcut}
                spellCheck={false}
                onChange={(event) => {
                  const hotkeys = draft.hotkeys.slice();
                  hotkeys[index] = {
                    ...hotkey,
                    shortcut: event.currentTarget.value,
                  };
                  setDraft({ ...draft, hotkeys });
                }}
              />
              <select
                aria-label="Action"
                value={hotkey.action}
                onChange={(event) => {
                  const hotkeys = draft.hotkeys.slice();
                  hotkeys[index] = {
                    ...hotkey,
                    action: event.currentTarget.value,
                  };
                  setDraft({ ...draft, hotkeys });
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
                className="remove"
                onClick={() =>
                  setDraft({
                    ...draft,
                    hotkeys: draft.hotkeys.filter((_, item) => item !== index),
                  })
                }
              >
                Remove
              </button>
            </div>
          ))}
          <button
            type="button"
            className="add"
            onClick={() =>
              setDraft({
                ...draft,
                hotkeys: [
                  ...draft.hotkeys,
                  {
                    id: crypto.randomUUID(),
                    shortcut: "",
                    action: "toggle_mute",
                  },
                ],
              })
            }
          >
            Add shortcut
          </button>
        </div>
        <p className="hint">
          Hold to mute puts the microphone back when the keys are released.
        </p>
        <label className="choice">
          <input
            type="checkbox"
            checked={draft.muteOnStart}
            onChange={(event) =>
              setDraft({ ...draft, muteOnStart: event.currentTarget.checked })
            }
          />
          Mute when MICRIX starts
        </label>
        <p className="hint">Applies the next time MICRIX starts.</p>
        <label className="choice">
          <input
            type="checkbox"
            checked={draft.startMinimized}
            onChange={(event) =>
              setDraft({
                ...draft,
                startMinimized: event.currentTarget.checked,
              })
            }
          />
          Start in the tray
        </label>
        <label className="choice">
          <input
            type="checkbox"
            checked={draft.closeToTray}
            onChange={(event) =>
              setDraft({ ...draft, closeToTray: event.currentTarget.checked })
            }
          />
          Hide when the window closes
        </label>
        <label className="choice">
          <input
            type="checkbox"
            checked={draft.startWithSystem}
            onChange={(event) =>
              setDraft({
                ...draft,
                startWithSystem: event.currentTarget.checked,
              })
            }
          />
          Start when Windows starts
        </label>
        <button type="submit" className="save">
          Save settings
        </button>
      </form>
    </main>
  );
}

export default App;
