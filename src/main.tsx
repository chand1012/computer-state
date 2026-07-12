import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";

const root = document.documentElement;
const media = window.matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () => root.classList.toggle("dark", media.matches);
applyTheme();
media.addEventListener("change", applyTheme);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <BrowserRouter><App /></BrowserRouter>
  </React.StrictMode>,
);
