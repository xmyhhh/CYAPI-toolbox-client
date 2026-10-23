/** Tutorials are published with integration catalogs and also used by the website. */
export interface IntegrationGuide {
  summary: string;
  outcome: string;
  preparation: string[];
  steps: { title: string; description: string }[];
  examples: { title: string; prompt: string; expected: string }[];
  limitations: string[];
  troubleshooting: { symptom: string; solution: string }[];
}

export interface SoftwareCompatibility {
  supported: string;
  minimumVersion?: string;
  maximumVersionExclusive?: string;
  testedVersions: string[];
  notes: string[];
}
