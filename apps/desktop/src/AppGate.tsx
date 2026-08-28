import { useEffect, useState } from "react";
import { App } from "./App";
import { LoginScreen } from "./components/LoginScreen";
import { fetchMe, type Me } from "@srelens/core";
import { isWeb } from "@srelens/core/platform";
import { LoadingState } from "./ui";

type Auth = { status: "pending" } | { status: "authed"; me: Me | null } | { status: "anon" };

/**
 * Auth gate in front of the app shell. On Tauri there is no server session to
 * probe — the desktop binary talks to the local kube API directly — so it
 * renders `<App/>` immediately with no network call. On web it probes
 * `fetchMe()` once: pending shows a spinner, null routes to `<LoginScreen/>`,
 * an identity renders `<App/>`.
 */
export default function AppGate() {
  const [auth, setAuth] = useState<Auth>(isWeb ? { status: "pending" } : { status: "authed", me: null });

  useEffect(() => {
    if (!isWeb) return;
    let cancelled = false;
    void fetchMe()
      .then((me) => !cancelled && setAuth(me ? { status: "authed", me } : { status: "anon" }))
      .catch(() => !cancelled && setAuth({ status: "anon" }));
    return () => {
      cancelled = true;
    };
  }, []);

  if (auth.status === "pending") {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <LoadingState label="Checking session…" />
      </div>
    );
  }
  if (auth.status === "anon") return <LoginScreen />;
  return <App />;
}
