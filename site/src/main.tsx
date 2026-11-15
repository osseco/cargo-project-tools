import { Package } from "lucide-react";
import React from "react";
import ReactDOM from "react-dom/client";
import {
  BrowserRouter,
  Navigate,
  Outlet,
  Route,
  Routes,
} from "react-router-dom";
import { AppShell } from "./components/app-shell";
import { GITHUB_REPO, LATEST_RELEASE } from "./lib/links";
import { DocRoute } from "./pages/doc";
import { HomePage } from "./pages/home";
import "./index.css";

const BRAND = "cargo project-tools";

const basename = (() => {
  const base = import.meta.env.BASE_URL;
  if (!base || base === "/") return undefined;
  return base.replace(/\/$/, "");
})();

function Layout() {
  return (
    <AppShell
      brand={BRAND}
      icon={<Package />}
      nav={[
        { href: "/", label: "Overview" },
        { href: "/docs", label: "Docs" },
        { href: GITHUB_REPO, label: "GitHub" },
      ]}
      footer={
        <>
          <a href={GITHUB_REPO}>GitHub</a>
          <a href={LATEST_RELEASE}>Releases</a>
          <a href={`${import.meta.env.BASE_URL}llms.txt`}>llms.txt</a>
        </>
      }
    >
      <Outlet />
    </AppShell>
  );
}

function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<HomePage />} />
        <Route path="/docs" element={<DocRoute />} />
        <Route path="/docs/:slug" element={<DocRoute />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Route>
    </Routes>
  );
}

const root = document.getElementById("root");
if (!root) {
  throw new Error("index.html has no #root element");
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
