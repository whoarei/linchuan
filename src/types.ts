export interface Device {
  id: string;
  name: string;
  platform: string;
  address: string;
  kind: string;
  host_device_id: string;
  public_key: string;
  capabilities: string[];
}
export interface FileMeta {
  id: string;
  name: string;
  size: number;
  mime: string;
}
export interface Transfer {
  id: string;
  message_id: string;
  group_id?: string;
  sender_id: string;
  sender_name: string;
  recipient_id: string;
  recipient_name: string;
  text: string;
  files: FileMeta[];
  status: string;
  bytes: number;
  total: number;
  created: number;
  created_at: number;
  error?: string;
}
export interface Conversation {
  id: string;
  principal_id: string;
  kind: "direct" | "group";
  peer_id?: string;
  peer_kind?: string | null;
  has_draft?: boolean;
  group_id?: string;
  name: string;
  preview: string;
  last_message_at: number;
  unread: number;
}
export interface GroupMember {
  device: Device;
  role: string;
  state: string;
  invite_id: string;
  sequence: number;
}
export interface Group {
  id: string;
  name: string;
  owner_id: string;
  revision: number;
  members: GroupMember[];
}
export interface MessageFile extends FileMeta {
  available: boolean;
  transfer_id: string;
}
export interface Message {
  id: string;
  position: number;
  sender_id: string;
  sender_name: string;
  text: string;
  created_at: number;
  direction: string;
  files: MessageFile[];
  deliveries: Transfer[];
}
export interface AppState {
  me: {
    id: string;
    name: string;
    platform: string;
    owner: boolean;
    auto_name?: string | null;
  };
  info: {
    id: string;
    name: string;
    addresses: string[];
    version: string;
    platform: string;
  };
  devices: Device[];
  conversations: Conversation[];
  groups: Group[];
  transfers: Transfer[];
  storage_used: number;
  storage_available: number | null;
  storage_headroom: number;
}
