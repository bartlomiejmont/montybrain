import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { mockProviders } from "../api/mock-runs";
import { mockSessionView } from "../api/mock-session";
import { AgentLimits, AgentTelemetry } from "../components/AgentTelemetry";
import { formatTokens, providerStatus, sessionConnection } from "./providerStatus";

describe("connection transparency", () => {
  it("distinguishes installation, authentication and an unknown check", () => {
    const p = mockProviders("ok")[1]!;
    expect(providerStatus(undefined).label).toBe("Sprawdzam…");
    expect(providerStatus({ ...p, available: false }).label).toBe("Brak instalacji");
    expect(providerStatus({ ...p, problem: "probe failed" }).label).toBe("Problem z CLI");
    expect(providerStatus({ ...p, auth: { ...p.auth, status: "logged_out" } }).label).toBe("Wylogowany");
    expect(providerStatus({ ...p, auth: { ...p.auth, status: "unknown" } }).label).toBe("Logowanie niepotwierdzone");
    expect(providerStatus(p).label).toBe("Zalogowany");
  });

  it("never labels an ended session as connected or missing usage as zero", () => {
    const v = mockSessionView("idle");
    expect(sessionConnection(v)).toBe("Połączono z CLI");
    expect(sessionConnection({ ...v, status: "stopped" })).toBe("Rozłączono");
    expect(sessionConnection({ ...v, status: "starting", telemetry: { ...v.telemetry, connected: false } })).toBe("Łączenie…");
    expect(formatTokens(null)).toBe("Brak danych");
    expect(formatTokens(0)).toBe("0");
    const html = renderToStaticMarkup(createElement(AgentTelemetry, { view: { ...v, telemetry: { connected: false, model: null, usage: null, limits: [] } } }));
    expect(html).toContain("Brak danych");
    expect(html).toContain("brak danych z CLI");
    expect(html).not.toContain("USD");
  });

  it("renders actual model, token counts and reported account limits", () => {
    const v = mockSessionView("idle");
    const html = renderToStaticMarkup(createElement(AgentTelemetry, { view: { ...v, telemetry: { ...v.telemetry, limits_at: 1800000000, limits: [{ id: "codex:primary", used_percent: 42, window_minutes: 300, resets_at: 1800000000 }] } } }));
    expect(html).toContain("claude-fable");
    expect(html).toContain(formatTokens(v.telemetry.usage!.input_tokens));
    expect(html).toContain('value="42"');
    expect(html).toContain("Odnowienie:");
    expect(html).toContain("nie jest opłatą za subskrypcję");
  });
});


it("never renders an untimestamped percentage and dates cached reports", () => {
  const v = mockSessionView("idle");
  v.telemetry.limits = [{ id: "codex:primary", used_percent: 42, window_minutes: 300, resets_at: null }];
  expect(renderToStaticMarkup(createElement(AgentTelemetry, { view: v }))).not.toContain('value="42"');
  const html = renderToStaticMarkup(createElement(AgentLimits, { snapshot: { limits: v.telemetry.limits, checked_at: 1800000000, status: null } }));
  expect(html).toContain('value="42"');
  expect(html).toContain("stan z");
  expect(html).toContain('dateTime="2027-01-15T08:00:00.000Z"');
});

it("renders the CLI's local reset date without inventing an epoch", () => {
  const html = renderToStaticMarkup(createElement(AgentLimits, { snapshot: { checked_at: 1800000000, status: null, limits: [{ id: "claude:five_hour", used_percent: 12, window_minutes: 300, resets_at: null, reset_label: "16.11, 01:19 (Europe/Warsaw)" }] } }));
  expect(html).toContain("12% wykorzystane");
  expect(html).toContain("Odnowienie: 16.11, 01:19 (Europe/Warsaw)");
  expect(html).toContain("stan z");
});
