<script setup lang="ts">
import { Globe, Monitor, Users } from "lucide-vue-next";
import type { Conversation } from "./types";
defineProps<{
  conversation: Conversation;
  label: string;
  selected: boolean;
  online: boolean;
}>();
defineEmits<{ select: [] }>();
function timestamp(ms: number) {
  const date = new Date(ms);
  return date.toDateString() === new Date().toDateString()
    ? date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}
</script>

<template>
  <button
    class="conversation-row"
    :class="{ selected }"
    :aria-label="`打开 ${label}`"
    @click="$emit('select')"
  >
    <span class="avatar" :class="{ group: conversation.kind === 'group' }">
      <Users v-if="conversation.kind === 'group'" :size="23" />
      <Globe v-else-if="conversation.peer_kind === 'browser'" :size="23" />
      <Monitor v-else :size="23" />
      <i v-if="conversation.kind === 'direct' && online" />
    </span>
    <span class="row-copy">
      <span class="row-title"
        ><b>{{ label }}</b
        ><time v-if="conversation.last_message_at">{{
          timestamp(conversation.last_message_at)
        }}</time></span
      >
      <span class="row-preview"
        ><span>{{
          conversation.preview ||
          (conversation.has_draft
            ? "[草稿]"
            : conversation.kind === "group"
              ? "群聊"
              : "还没有传输记录")
        }}</span
        ><em v-if="conversation.unread">{{
          conversation.unread > 99 ? "99+" : conversation.unread
        }}</em></span
      >
    </span>
  </button>
</template>
