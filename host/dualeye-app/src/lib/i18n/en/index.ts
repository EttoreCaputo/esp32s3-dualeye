// The app's texts in English: the reference every other language mirrors.
import faces from "./faces";
import main from "./main";
import pet from "./pet";
import settings from "./settings";

const en = { main, faces, pet, settings };
export type Messages = typeof en;
export default en;
