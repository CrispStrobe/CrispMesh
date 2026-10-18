import { render } from "preact";
import { App } from "./ui/app";
import { applyTheme, loadPrefs } from "./ui/prefs";
import { resolveLanguage, setLanguage } from "./i18n";
import "./styles.css";

applyTheme(loadPrefs().theme);
setLanguage(resolveLanguage(loadPrefs().language));
render(<App />, document.getElementById("app")!);
