export type WorkflowMode = 'mock' | 'local-billing';

export interface WorkflowPrice {
  amount: number;
  currency: string;
  unit: string;
}

export interface WorkflowDisplay {
  label?: string;
  icon?: string;
  input: string;
  output: string;
  steps: string[];
  note?: string;
  price?: WorkflowPrice;
}

export interface WorkflowCategory {
  id: string;
  label: string;
  scope: 'all' | 'category' | 'personal';
  templateIds: string[];
}

export interface WorkflowTemplate {
  id: string;
  name: string;
  summary: string;
  category: string;
  cover: string;
  version: string;
  requiresReferences: boolean;
  source: 'server' | 'personal';
  display?: WorkflowDisplay;
  apiGraph?: Record<string, unknown>;
  workflow?: { nodes: { id: string; pos: number[]; size: number[] }[] };
  createdAt?: number;
  revision?: number;
}

export interface WorkflowDraft {
  id: string;
  templateId: string;
  templateVersion: string;
  name: string;
  cover: string;
  requiresReferences: boolean;
  display?: WorkflowDisplay;
  description: string;
  references: string[];
  apiGraph?: Record<string, unknown>;
  workflow?: WorkflowTemplate['workflow'];
  audio?: string;
  createdAt: number;
  updatedAt: number;
  revision: number;
  taskId?: string;
  guest?: boolean;
}

export type WorkflowStatus =
  | 'reserving'
  | 'submitting'
  | 'queued'
  | 'running'
  | 'collecting'
  | 'settling'
  | 'releasing'
  | 'reservation_unknown'
  | 'billing_unknown'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'interrupted'
  | 'unknown';

export interface WorkflowOutput {
  id: string;
  title?: string;
  kind?: 'image' | 'video' | 'mesh' | 'text';
  preview?: string;
  url: string;
  download: string;
}

export interface WorkflowStep {
  id: string;
  title: string;
  kind: string;
  status: 'waiting' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'cancelled';
  startedAt?: number;
  finishedAt?: number;
}

export interface WorkflowQuote {
  mode: WorkflowMode;
  draftId?: string;
  revision?: number;
  amount: number;
  unit: string;
  items?: { id: string; title: string; amount: number }[];
  token?: string;
  bridgeQuote?: unknown;
  parametersHash?: string;
}

export interface WorkflowTask {
  id: string;
  templateId: string;
  templateVersion: string;
  name: string;
  cover: string;
  requiresReferences: boolean;
  display?: WorkflowDisplay;
  description: string;
  references: string[];
  createdAt: number;
  updatedAt: number;
  status: WorkflowStatus;
  outputs: WorkflowOutput[];
  message: string;
  steps?: WorkflowStep[];
  quote?: WorkflowQuote;
  billing?: { mode: string; status: string; amount: number; unit: string };
  finishedAt?: number;
}

export interface WorkflowCatalog {
  categories: WorkflowCategory[];
  templates: WorkflowTemplate[];
  mode: WorkflowMode;
  catalogRevision?: string;
}

export interface WorkflowState extends WorkflowCatalog {
  drafts: WorkflowDraft[];
  tasks: WorkflowTask[];
}
