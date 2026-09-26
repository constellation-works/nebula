import { useEffect, useState } from "react";
import * as api from "./api";
import { GraphView } from "./GraphView";
import { InboxView } from "./InboxView";
import { useInbox } from "./useInbox";

type Tab = "inbox" | "graph" | "settings";

function SettingsPanel() {
  const [shortcut, setShortcut] = useState("");
  const [login, setLogin] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loginAvailable, setLoginAvailable] = useState(false);
  const [savingShortcut, setSavingShortcut] = useState(false);
  const [savingLogin, setSavingLogin] = useState(false);
  const [shortcutMessage, setShortcutMessage] = useState("");
  const [loginMessage, setLoginMessage] = useState("");

  useEffect(() => {
    let active = true;
    void Promise.allSettled([api.captureShortcut(), api.launchAtLogin()]).then(([shortcutResult, loginResult]) => {
      if (!active) return;
      if (shortcutResult.status === "fulfilled") setShortcut(shortcutResult.value);
      else setShortcutMessage(`Could not load capture shortcut: ${String(shortcutResult.reason)}`);
      if (loginResult.status === "fulfilled") {
        setLogin(loginResult.value);
        setLoginAvailable(true);
      } else setLoginMessage(`Could not load launch at login: ${String(loginResult.reason)}`);
      setLoading(false);
    });
    return () => { active = false; };
  }, []);

  const saveShortcut = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSavingShortcut(true);
    setShortcutMessage("");
    try {
      const saved = await api.setCaptureShortcut(shortcut);
      setShortcut(saved);
      setShortcutMessage("Capture shortcut saved and active.");
    } catch (error) {
      setShortcutMessage(`Shortcut not changed: ${String(error)}`);
    } finally {
      setSavingShortcut(false);
    }
  };

  const changeLogin = async (enabled: boolean) => {
    setSavingLogin(true);
    setLoginMessage("");
    try {
      setLogin(await api.setLaunchAtLogin(enabled));
      setLoginMessage(enabled ? "Launch at login enabled." : "Launch at login disabled.");
    } catch (error) {
      setLoginMessage(`Launch at login not changed: ${String(error)}`);
    } finally {
      setSavingLogin(false);
    }
  };

  return (
    <section className="settings" aria-label="Settings">
      <h1>Settings</h1>
      <form onSubmit={(event) => void saveShortcut(event)}>
        <label htmlFor="capture-shortcut">Capture shortcut</label>
        <div className="settings__shortcut">
          <input
            id="capture-shortcut"
            value={shortcut}
            onChange={(event) => setShortcut(event.target.value)}
            disabled={loading || savingShortcut}
            placeholder="Alt+Space"
          />
          <button type="submit" disabled={loading || savingShortcut}>Save shortcut</button>
        </div>
        <p className="settings__hint">Use a modifier and key, for example Alt+Space or CmdOrCtrl+Shift+N.</p>
        {shortcutMessage && <p role="status" className="settings__message">{shortcutMessage}</p>}
      </form>
      <label className="settings__login">
        <input
          type="checkbox"
          checked={login}
          onChange={(event) => void changeLogin(event.target.checked)}
          disabled={loading || savingLogin || !loginAvailable}
        />
        Launch at login
      </label>
      {loginMessage && <p role="status" className="settings__message">{loginMessage}</p>}
    </section>
  );
}

/** The main window: corpus views and application settings. */
export function App() {
  const [tab, setTab] = useState<Tab>("inbox");
  const [graphVisited, setGraphVisited] = useState(false);
  const [settingsVisited, setSettingsVisited] = useState(false);
  const [startupWarnings, setStartupWarnings] = useState<string[]>([]);
  const inbox = useInbox();

  useEffect(() => {
    let active = true;
    void api.startupWarnings()
      .then((warnings) => {
        if (active) setStartupWarnings(warnings);
      })
      .catch((error: unknown) => {
        if (active) setStartupWarnings([`Could not load startup warnings: ${String(error)}`]);
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let active = true;
    let dispose: (() => void) | undefined;
    void api.onOpenSettings(() => {
      if (!active) return;
      setSettingsVisited(true);
      setTab("settings");
    }).then((off) => {
      if (active) dispose = off;
      else off();
    }).catch((error: unknown) => {
      if (active) setStartupWarnings((warnings) => [...warnings, `Could not open Settings from the tray: ${String(error)}`]);
    });
    return () => { active = false; dispose?.(); };
  }, []);

  return (
    <div className="app">
      {startupWarnings.length > 0 && (
        <section className="startup-warning" role="alert" aria-labelledby="startup-warning-title">
          <h2 id="startup-warning-title">Startup warning</h2>
          <ul>
            {startupWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </section>
      )}
      <nav className="tabs" role="tablist">
        <button
          role="tab"
          type="button"
          className="tab"
          aria-selected={tab === "inbox"}
          onClick={() => setTab("inbox")}
        >
          Inbox
          {inbox.entries.length > 0 && <span className="tab__count">{inbox.entries.length}</span>}
        </button>
        <button
          role="tab"
          type="button"
          className="tab"
          aria-selected={tab === "graph"}
          onClick={() => {
            setGraphVisited(true);
            setTab("graph");
          }}
        >
          Graph
        </button>
        <button
          role="tab"
          type="button"
          className="tab"
          aria-selected={tab === "settings"}
          onClick={() => {
            setSettingsVisited(true);
            setTab("settings");
          }}
        >
          Settings
        </button>
      </nav>
      <main className="view" hidden={tab !== "inbox"}>
        <InboxView inbox={inbox} />
      </main>
      {graphVisited && (
        <main className="view" hidden={tab !== "graph"}>
          <GraphView active={tab === "graph"} />
        </main>
      )}
      {settingsVisited && (
        <main className="view" hidden={tab !== "settings"}>
          <SettingsPanel />
        </main>
      )}
    </div>
  );
}
