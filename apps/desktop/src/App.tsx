import { useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Snapshot = {
  status: string;
  error: string;
  settings_json: string;
};

type Draft = {
  shortcut: string;
  muteOnStart: boolean;
  startMinimized: boolean;
  closeToTray: boolean;
  startWithSystem: boolean;
};

const EMPTY_DRAFT: Draft = {
  shortcut: "F9",
  muteOnStart: false,
  startMinimized: true,
  closeToTray: true,
  startWithSystem: false,
};

function readDraft(settingsJson: string): Draft {
  const value = JSON.parse(settingsJson) as {
    microphone?: { mute_on_start?: boolean };
    hotkeys?: { shortcut?: string; action?: string; enabled?: boolean }[];
    general?: {
      start_minimized?: boolean;
      close_to_tray?: boolean;
      start_with_system?: boolean;
    };
  };
  const hotkey = value.hotkeys?.find(
    (item) => item.action === "toggle_mute" && item.enabled !== false,
  );
  return {
    shortcut: hotkey?.shortcut ?? "",
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
      hotkeys: [
        {
          id: "toggle-default-mic",
          shortcut: draft.shortcut.trim(),
          action: "toggle_mute",
          enabled: true,
        },
      ],
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
        <label>
          Shortcut
          <input
            value={draft.shortcut}
            spellCheck={false}
            onChange={(event) =>
              setDraft({ ...draft, shortcut: event.currentTarget.value })
            }
          />
        </label>
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
        <button type="submit" className="save">
          Save settings
        </button>
      </form>
    </main>
  );
}

export default App;
