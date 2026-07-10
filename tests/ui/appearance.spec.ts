import { expect, test } from "playwright/test";

test.describe("public Lab appearance", () => {
  test("keeps public navigation and accessible wallet control at every viewport", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "automaton lab" })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Public Lab sections" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Fleet" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Room" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Spawn" })).toBeVisible();
    await expect(page.getByRole("button", { name: /Wallet/ })).toBeVisible();
    await page.getByRole("button", { name: /Wallet/ }).focus();
    await expect(page.getByRole("button", { name: /Wallet/ })).toBeFocused();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });
});

test.describe("operator appearance", () => {
  test("keeps operator identity and stop control separate from Lab", async ({ browserName, page }) => {
    await page.goto("http://127.0.0.1:4173");
    await expect(page.getByText("Operator / Evaluation")).toBeVisible();
    await expect(page.getByRole("button", { name: /Stop Run/ })).toBeVisible();
    expect(await page.locator("[data-ui-theme=operator]").count()).toBe(1);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    void browserName;
  });
});
