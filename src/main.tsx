import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";
import "./views";
import { App } from "./App";
import { VaultProvider } from "./store/vault";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <VaultProvider>
      <App />
    </VaultProvider>
  </StrictMode>,
);
