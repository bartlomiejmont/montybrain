// Entry of the quick-capture window (quick.html → src-tauri native.rs `QUICK`).

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "../styles.css";
import "./quick.css";
import { QuickCapture } from "./QuickCapture";

createRoot(document.getElementById("quick-root")!).render(
  <StrictMode>
    <QuickCapture />
  </StrictMode>,
);
