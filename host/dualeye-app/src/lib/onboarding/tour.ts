// The tour's steps, in order. Each points at the element with
// `data-tour="<target>"`; a step without a target (or whose element isn't on
// screen, like the pet before the board says it has one) shows in the middle.
// Its texts are `onboarding.tour.steps[id]`.
//
// A step new in a later app version gets `since` = that tour version, and
// TOUR_VERSION goes up: people who saw the older tour are offered just those.

export const TOUR_VERSION = 1;

export type TourId = "faces" | "telemetry" | "pet" | "timers" | "settings" | "tray";
export type TourStep = { id: TourId; target?: string; since: number };

export const TOUR: TourStep[] = [
  { id: "faces", target: "board", since: 1 },
  { id: "telemetry", target: "telemetry", since: 1 },
  { id: "pet", target: "pet", since: 1 },
  { id: "timers", target: "board", since: 1 },
  { id: "settings", target: "settings", since: 1 },
  { id: "tray", since: 1 },
];
