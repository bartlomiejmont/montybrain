import { beforeEach } from "vitest";
import { setLanguage } from "../i18n";

// Existing fixtures exercise the Polish interface. The i18n suite explicitly
// covers the English default and switching back and forth.
beforeEach(() => setLanguage("pl"));
