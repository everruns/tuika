/**
 * Copy-on-click for the hero install command.
 *
 * Selecting the text still works, so the button is a convenience rather than
 * the only way to get the command out of the page. A failed copy (insecure
 * context, denied permission) says so instead of silently pretending success.
 */

const RESET_MS = 1600;

function initHomeInstall(button: HTMLButtonElement): void {
  const label = button.querySelector<HTMLElement>("[data-home-install-label]");
  const status = document.querySelector<HTMLElement>("[data-home-install-status]");
  const defaultLabel = label?.textContent ?? "Install";
  let timer: number | undefined;

  button.addEventListener("click", async () => {
    let state: "copied" | "failed" = "copied";
    try {
      await navigator.clipboard.writeText(button.dataset.command ?? "");
    } catch {
      state = "failed";
    }

    button.dataset.state = state;
    if (label) label.textContent = state === "copied" ? "Copied" : "Press ⌘C";
    if (status) status.textContent = state === "copied" ? "Copied to clipboard" : "Copy failed — select the command and copy it manually";

    if (timer) window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      delete button.dataset.state;
      if (label) label.textContent = defaultLabel;
      if (status) status.textContent = "";
    }, RESET_MS);
  });
}

document.querySelectorAll<HTMLButtonElement>("[data-home-install]").forEach(initHomeInstall);
