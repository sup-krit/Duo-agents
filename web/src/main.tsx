import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Route, Routes, useParams } from "react-router";
import "./globals.css";
import type { SessionSummary } from "./types";
import { StartForm } from "./duo/start-form";
import { SessionView } from "./duo/session-view";

function Home() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  useEffect(() => {
    fetch("/api/sessions").then((r) => r.json()).then((s) => Array.isArray(s) && setSessions(s)).catch(() => {});
  }, []);
  return <StartForm sessions={sessions} />;
}

function Session() {
  const { id = "" } = useParams();
  return <SessionView id={id} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<Home />} />
        <Route path="/s/:id" element={<Session />} />
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
