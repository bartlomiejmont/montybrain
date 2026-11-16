import { LanguageSync } from "./i18n/LanguageSync";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";
import "./components/project-colors.css";
import "./views";
import { App } from "./App";
import { VaultProvider } from "./store/vault";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LanguageSync />
    <VaultProvider>
      <App />
    </VaultProvider>
  </StrictMode>,
);
