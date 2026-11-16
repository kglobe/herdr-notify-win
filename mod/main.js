// Replaces Herdr's in-app toast: every notify() becomes a Windows toast.
// herdr-toast.exe (always running, see install.ps1) shows it and runs the action on click.

import { View, div } from "gpui-kit";
import { props, closeOverlay, writeTextFile, home, checking, agents, panes } from "herdr";

export default class WinToast extends View {
  init(_props, cx) {
    const p = props() ?? {};
    if (!checking()) {
      // line is herdr-app.sock's request format, sent as-is when the toast is clicked.
      const line = p.action?.name ? JSON.stringify({ do: p.action.name, args: p.action.args ?? {} }) : "";
      // Who sent it, for the toast's header, summary and footer.
      const t = p.action?.args?.terminal;
      const a = t ? agents().find((x) => x.address === t) : null;
      const pane = t ? panes().find((x) => x.terminal === t) : null;
      const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
      writeTextFile(`${home()}/AppData/Local/herdr-win-toast/spool/${id}.json`,
        JSON.stringify({
          title: p.title ?? "", body: p.body ?? "", line,
          app: a?.app ?? "", space: a?.space_name ?? "", tab: a?.tab_name ?? "", term_title: a?.title ?? "",
          cwd: pane?.cwd ?? "", branch: pane?.branch ?? "",
        }), null).catch(() => {});
    }
    if (p.overlay) closeOverlay(p.overlay);
  }

  render() {
    return div();
  }
}
