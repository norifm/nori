// Signs a check's page in: the welcome's server picked on a fresh install, then the sign-in form the server's
// address fills in, then the first run's questions skipped to their end.

/** Signs `page` in unless it opened a profile, and waits for Home. */
export async function signIn(page, timeout = 30000) {
  await page.waitForFunction(() => window.nori.page === "home" || document.querySelector("#welcome, #login"), { timeout });
  if (await page.$("#welcome")) {
    await page.click("#welcome [data-kind=subsonic]");
    await page.click("#welcome-continue");
    await page.waitForSelector("#login");
  }
  if (await page.$("#login")) await page.click("#login .pill");
  await page.waitForFunction(() => window.nori.page === "home", { timeout });
  while (await page.$("#setup-skip")) await page.click("#setup-skip");
  if (await page.$("#setup-start")) await page.click("#setup-start");
}
