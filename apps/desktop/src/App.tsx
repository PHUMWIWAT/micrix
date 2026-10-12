import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type ShortcutStatus = {
  shortcut: string;
  error: string;
};

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");
  const [shortcut, setShortcut] = useState("");
  const [hotkeyError, setHotkeyError] = useState("");

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    invoke<ShortcutStatus>("shortcut_status")
      .then((status) => {
        if (cancelled) return;
        setShortcut(status.shortcut);
        setHotkeyError(status.error);
      })
      .catch((error: unknown) => {
        if (!cancelled) setHotkeyError(String(error));
      });

    listen<string>("hotkey-error", (event) => {
      setHotkeyError(event.payload);
    }).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="container">
      <img src="/logo.png" className="logo" alt="MICRIX" />
      <h1>MICRIX</h1>
      <p>Toggle mute: {shortcut || "—"}</p>
      {hotkeyError ? (
        <p className="notice" role="alert">
          {hotkeyError}
        </p>
      ) : null}

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>
    </main>
  );
}

export default App;
