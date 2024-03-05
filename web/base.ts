export enum ToolKind {
  None,
  Dispatch,
  Help,
  Search,
}

// We need to propagate some settings from the specific app build into this
// framework.

export interface BuildSpecificSettings {
  indexUrl: string;
}

export const buildSpecificSettings: BuildSpecificSettings = {
  indexUrl: ""
};
