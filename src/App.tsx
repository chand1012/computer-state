import { lazy, Suspense, useEffect } from "react";
import { Activity, Settings } from "lucide-react";
import { NavLink, Navigate, Route, Routes, useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import pulseCoreLogo from "./assets/pulse-core-logo-128.png";
import "./App.css";

const MetricsPage = lazy(() => import("./features/metrics/MetricsPage").then((module) => ({ default: module.MetricsPage })));
const SettingsPage = lazy(() => import("./features/settings/SettingsPage").then((module) => ({ default: module.SettingsPage })));

export default function App() {
  const navigate = useNavigate();

  useEffect(() => {
    let disposed = false;
    let stop: undefined | (() => void);
    listen<string>("navigation://requested", ({ payload }) => {
      if (payload === "/metrics" || payload === "/settings") navigate(payload);
    }).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    return () => { disposed = true; stop?.(); };
  }, [navigate]);

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="brand">
          <div className="brand-mark"><img src={pulseCoreLogo} alt="" /></div>
          <div>
            <strong>Computer State</strong>
            <span>Local system telemetry</span>
          </div>
        </div>
        <nav className="tabs" aria-label="Primary navigation">
          <NavLink to="/metrics" className={({ isActive }) => `tab ${isActive ? "active" : ""}`}>
            <Activity size={15} /> Metrics
          </NavLink>
          <NavLink to="/settings" className={({ isActive }) => `tab ${isActive ? "active" : ""}`}>
            <Settings size={15} /> Settings
          </NavLink>
        </nav>
      </header>
      <main className="page">
        <Suspense fallback={<div className="center-state"><div className="spinner" />Loading view…</div>}>
          <Routes>
            <Route path="/metrics" element={<MetricsPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="*" element={<Navigate to="/metrics" replace />} />
          </Routes>
        </Suspense>
      </main>
    </div>
  );
}
