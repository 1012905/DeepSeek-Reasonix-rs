import type { Messages } from "./types";

export const RELAY: Messages = {
  "relay.code_expires": { zh: "还有 {n} 分钟有效", en: "Valid for {n} more minutes" },
  "relay.code_label": { zh: "配对码", en: "Pairing code" },
  "relay.code_placeholder": { zh: "六位数字", en: "Six digits" },
  "relay.connect": { zh: "连接", en: "Connect" },
  "relay.connected": { zh: "已连接", en: "Connected" },
  "relay.connecting": { zh: "正在连接…", en: "Connecting…" },
  "relay.err_ended": { zh: "桌面端已断开这次连接，请重新获取配对码。", en: "The desktop closed this session. Ask it for a new pairing code." },
  "relay.err_invalid_code": { zh: "配对码无效或已过期，请核对这六位数字。", en: "That code is unknown or has expired. Check the six digits and try again." },
  "relay.err_offline": { zh: "连不上中继服务，请检查网络后重试。", en: "The relay cannot be reached. Check the connection and try again." },
  "relay.err_protocol": { zh: "网页和桌面端未能完成握手，请更新 Reasonix Studio 或刷新本页。", en: "The page and the desktop could not complete the handshake. Update Reasonix Studio or reload this page." },
  "relay.offline": { zh: "离线", en: "Offline" },
  "relay.online": { zh: "在线", en: "Online" },
  "relay.pair_hint": { zh: "输入 Reasonix Studio 中「设置 → 远程 → 网页访问」里显示的配对码。", en: "Enter the pairing code shown in Reasonix Studio under Settings → Remote → Web access." },
  "relay.pair_title": { zh: "连接到这台电脑", en: "Connect to this desktop" },
  "relay.show_code": { zh: "显示配对码", en: "Show pairing code" },
  "relay.turn_off": { zh: "关闭", en: "Turn off" },
  "relay.turn_on": { zh: "开启", en: "Turn on" },
  "relay.web_access": { zh: "网页访问", en: "Web access" },
  "relay.web_access_hint": { zh: "在浏览器里打开 Reasonix 网页版，输入配对码即可访问这台电脑。", en: "Open the Reasonix web app in a browser and enter a pairing code to reach this computer." },
};
