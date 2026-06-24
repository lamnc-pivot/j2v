export interface ModelInfo {
  id: string;
  name: string;
  installed: boolean;
  installing?: boolean;
  error?: string;
}

export interface ModelStatusResponse {
  id: string;
  name: string;
  installed: boolean;
}
