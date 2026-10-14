// The app's texts in English: the reference every other language mirrors.
import faces from "./faces";
import main from "./main";
import onboarding from "./onboarding";
import pet from "./pet";
import settings from "./settings";

const en = { main, faces, pet, settings, onboarding };
export type Messages = typeof en;
export default en;
