# Browser companion installation

Start the OpenDownload desktop application on Windows or Linux at its default port, 8765. The browser must run on the same machine. There are no third-party services or accounts.

## Chrome, Chromium, Brave, Edge

1. Open `chrome://extensions` (or your browser's extensions page).
2. Enable Developer mode and select **Load unpacked**.
3. Select the bundled `extensions/chromium` directory, or this repository's `extension` directory, containing `manifest.json`.
4. In the desktop app, open **Browser companion** and choose **Copy pairing code**. The optional headless dashboard also has a **Browser pairing** button.
5. Open the extension popup, paste the token, and choose **Save pairing**.

If your browser administrator blocks unpacked extensions, installation requires an administrator-approved distribution method. This project does not bypass that policy.

## Firefox

Run `python3 extension/package.py`. Open `about:debugging`, choose **This Firefox → Load Temporary Add-on**, and select `dist/opendownload-firefox.zip`. Pair using the same steps above. Temporary add-ons disappear on browser restart. Permanent distribution requires Mozilla signing, which has not been performed. The Firefox manifest targets Firefox 128+; actual Firefox operation has not been validated in the onboarding machine.

## Use

On a YouTube or other supported video page, open the popup and choose **Download video from this page**. You can also right-click a page, link, or media element and select **Download with OpenDownload**, paste a URL, or use the scanner (up to 200 HTTP(S) URLs in the top frame). The desktop app extracts supported video pages and opens a separate progress window. Unsupported pages report an error. Protected videos, blob URLs, and browser-only login sessions are unsupported.

**Send new browser downloads to the app** is optional and off by default. It pauses a browser transfer before handoff and cancels that transfer only after the app accepts the URL. A failed handoff resumes the browser transfer; already completed downloads stay in the browser. The app may subsequently fail because it lacks a site's browser cookies or because a URL expires. In that case, disable capture and retry in the browser. A badge marked `!` means the popup has a message to review.

Pairing is remembered after restarting the app. Re-pair if its bridge-key file is deleted or you choose a different state file. Treat the pairing token as a password: anyone with it and local access can queue downloads. The token is stored in extension local storage and never sent beyond `127.0.0.1:8765`. Custom app ports are not supported by this version of the companion.

## Permissions

- `storage`: save pairing and the opt-in capture preference.
- `contextMenus`: add the download action.
- `activeTab` and `scripting`: scan the current page after your click; no background scanning or broad website access.
- `downloads`: pause, cancel, or resume browser downloads only when capture is enabled.
- Loopback host access: communicate with the local app.

Cookies, login credentials, browser history, and remote analytics are not collected or forwarded. Extension archives are unsigned development builds. No browser-store listing is claimed.
