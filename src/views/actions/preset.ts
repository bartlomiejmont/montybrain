// Opening „Akcje" with a recipe and parameters chosen elsewhere (e.g. the
// project view's update menu): the form picks the preset up once.

import { writePref } from "../../store/ui";

export const RECIPE_PREF = "montybrain.akcje.recipe";

const presets = new Map<string, Record<string, string>>();

/** Selects `recipe` in Akcje and pre-fills `params` the next time its form opens. */
export function presetAction(recipe: string, params: Record<string, string>) {
  writePref(RECIPE_PREF, recipe);
  presets.set(recipe, params);
}

/** The preset of `recipe`, once. */
export function takePreset(recipe: string): Record<string, string> | null {
  const p = presets.get(recipe) ?? null;
  presets.delete(recipe);
  return p;
}
