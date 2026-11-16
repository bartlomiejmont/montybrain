import { createContext, useContext } from "react";

import type { DetailTarget } from "../lib/details";

/** Shared selection for a workspace and its side panel. */
export const DetailContext = createContext<((target: DetailTarget) => void) | null>(null);
export const useOpenDetail = () => useContext(DetailContext);
