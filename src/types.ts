export interface Device { id: string; name: string; platform: string; address: string; kind: 'app'|'browser'|'host'; }
export interface FileMeta { id: string; name: string; size: number; mime: string; }
export interface Transfer { id: string; sender_id: string; sender_name: string; recipient_id: string; recipient_name: string; text: string; files: FileMeta[]; status: string; bytes: number; total: number; created: number; error?: string; }
export interface AppState { me: { id: string; name: string; owner: boolean }; info: { name: string; addresses: string[]; version: string }; devices: Device[]; transfers: Transfer[]; storage_used: number; storage_limit: number; }
