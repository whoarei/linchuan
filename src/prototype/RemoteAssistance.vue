<script setup lang="ts">
import { computed, ref } from "vue";
import {
  AppWindow, Check, CircleStop, Eye, Hand, Laptop, Monitor,
  MonitorUp, MousePointer2, ShieldCheck, X,
} from "lucide-vue-next";
import DemoDesktop from "./DemoDesktop.vue";

type Role = "sharer" | "helper";
type Source = "desktop" | "window";
type Phase = "idle" | "invited" | "watching" | "requested" | "controlling" | "ended";

const devices = [
  { role: "sharer", name: "白鹭台式机", label: "分享方", icon: Monitor },
  { role: "helper", name: "松果笔记本", label: "协助方", icon: Laptop },
] as const;
const sources = [
  { id: "desktop", label: "整个桌面", detail: "分享屏幕上的所有窗口", icon: Monitor },
  { id: "window", label: "协作便笺窗口", detail: "只分享这个窗口", icon: AppWindow },
] as const;

const role = ref<Role>("sharer");
const phase = ref<Phase>("idle");
const source = ref<Source>("desktop");
const draftSource = ref<Source>("desktop");
const sourceDialog = ref<HTMLDialogElement>();
const note = ref("");
const noteOpen = ref(true);
const feedback = ref("");
const endReason = ref("");

const isSharer = computed(() => role.value === "sharer");
const peer = computed(() => devices[isSharer.value ? 1 : 0].name);
const sourceLabel = computed(() => sources[source.value === "desktop" ? 0 : 1].label);
const joined = computed(() => ["watching", "requested", "controlling"].includes(phase.value));
const screenVisible = computed(() => joined.value || (isSharer.value && phase.value === "invited"));
const canOperate = computed(() => isSharer.value || phase.value === "controlling");

const banner = computed(() => {
  switch (phase.value) {
    case "invited":
      return { title: `邀请已发送给${peer.value}`, detail: "对方加入后即可观看画面。", icon: MonitorUp };
    case "requested":
      return isSharer.value
        ? { title: `${peer.value}请求控制`, detail: "允许后，对方可以使用鼠标和键盘。", icon: Hand }
        : { title: "已发送控制请求", detail: "等待分享方确认，期间可以继续观看。", icon: Hand };
    case "controlling":
      return isSharer.value
        ? { title: `${peer.value}正在控制`, detail: "对方可以操作鼠标和键盘，你可以随时收回。", icon: MousePointer2 }
        : { title: "你正在控制共享画面", detail: "试着点击协作便笺，或在里面输入文字。", icon: MousePointer2 };
    default:
      return isSharer.value
        ? { title: "你正在分享画面", detail: `${peer.value}可以观看${sourceLabel.value}。`, icon: MonitorUp }
        : { title: `正在观看${peer.value}的画面`, detail: feedback.value || "需要操作时，可以请求控制。", icon: Eye };
  }
});

function pickSource() {
  draftSource.value = source.value;
  sourceDialog.value?.showModal();
}

function startSharing() {
  source.value = draftSource.value;
  note.value = "共享便笺\n\n接管后，可以在这里试写一行。";
  noteOpen.value = true;
  feedback.value = "";
  phase.value = "invited";
  sourceDialog.value?.close();
}

function returnToWatching(message = "") {
  feedback.value = message;
  phase.value = "watching";
}

function endSession(reason: string) {
  endReason.value = reason;
  phase.value = "ended";
}
</script>

<template>
  <div class="assist-page">
    <header class="assist-topbar">
      <div class="assist-topbar-inner">
        <div class="assist-brand">
          <span class="assist-brand-mark" aria-hidden="true">邻</span>
          <b>邻传</b>
          <span class="assist-prototype-label">交互原型</span>
        </div>
        <div class="assist-perspective">
          <span aria-hidden="true">切换视角</span>
          <div class="assist-roles" role="group" aria-label="切换视角">
            <button
              v-for="device in devices"
              :key="device.role"
              type="button"
              :aria-pressed="role === device.role"
              @click="role = device.role"
            >
              <component :is="device.icon" :size="15" />{{ device.label }}
            </button>
          </div>
        </div>
      </div>
    </header>

    <main class="assist-workspace">
      <div class="assist-heading">
        <h1>远程协助</h1>
        <p>与 {{ peer }} 的一对一会话</p>
      </div>

      <div class="assist-layout">
        <div class="assist-main">
          <div
            v-if="screenVisible"
            class="assist-banner"
            :class="{ 'needs-attention': phase === 'requested' || phase === 'controlling' }"
          >
            <div class="assist-banner-message" role="status">
              <component :is="banner.icon" :size="20" />
              <div><strong>{{ banner.title }}</strong><p>{{ banner.detail }}</p></div>
            </div>
            <div class="assist-actions">
              <template v-if="isSharer && phase === 'requested'">
                <button class="secondary" @click="returnToWatching('分享方拒绝了控制请求，你可以继续观看。')">拒绝</button>
                <button class="primary" @click="phase = 'controlling'">允许控制</button>
              </template>
              <button
                v-if="isSharer && phase === 'controlling'"
                class="primary"
                @click="returnToWatching('分享方已收回控制，你可以继续观看。')"
              >收回控制</button>
              <button v-if="isSharer" class="assist-stop" @click="endSession('分享方已停止分享。')">
                <CircleStop :size="15" />停止分享
              </button>
              <template v-else>
                <button v-if="phase === 'watching'" class="primary" @click="phase = 'requested'">
                  <MousePointer2 :size="15" />请求控制
                </button>
                <button v-else-if="phase === 'requested'" class="secondary" @click="returnToWatching()">取消请求</button>
                <button v-else class="primary" @click="returnToWatching('已交还控制，你可以继续观看。')">交还控制</button>
                <button class="secondary" @click="endSession('协助方已离开会话。')">离开会话</button>
              </template>
            </div>
          </div>

          <section class="assist-card assist-screen" aria-labelledby="assist-screen-title">
            <header class="assist-card-heading">
              <h2 id="assist-screen-title"><Monitor :size="17" />共享画面</h2>
              <span v-if="screenVisible" class="assist-source">
                <i aria-hidden="true" />{{ sourceLabel }}
              </span>
            </header>
            <div v-if="screenVisible" class="assist-stage">
              <DemoDesktop
                v-model:note="note"
                v-model:note-open="noteOpen"
                :window-only="source === 'window'"
                :can-operate="canOperate"
              />
            </div>
            <div v-else class="assist-stage assist-empty">
              <div class="assist-empty-icon" aria-hidden="true">
                <CircleStop v-if="phase === 'ended'" :size="30" :stroke-width="1.6" />
                <MonitorUp v-else :size="32" :stroke-width="1.6" />
              </div>
              <template v-if="phase === 'ended'">
                <h3>会话已结束</h3>
                <p>{{ endReason }}</p>
                <button v-if="isSharer" class="primary" @click="pickSource">再次分享</button>
              </template>
              <template v-else-if="phase === 'invited'">
                <h3>{{ peer }}邀请你观看</h3>
                <p>对方想向你分享{{ sourceLabel }}。</p>
                <div class="assist-actions">
                  <button class="secondary" @click="endSession('协助方拒绝了邀请。')">拒绝邀请</button>
                  <button class="primary" @click="returnToWatching()">加入观看</button>
                </div>
              </template>
              <template v-else-if="isSharer">
                <h3>分享你的画面</h3>
                <p>选择整个桌面或一个窗口，邀请{{ peer }}观看。</p>
                <button class="primary" @click="pickSource"><MonitorUp :size="16" />分享桌面</button>
              </template>
              <template v-else>
                <h3>等待分享</h3>
                <p>收到{{ peer }}的邀请后，你可以加入观看。</p>
              </template>
            </div>
          </section>
        </div>

        <aside class="assist-sidebar" aria-label="会话详情">
          <section class="assist-card assist-details" aria-labelledby="assist-participants">
            <h2 id="assist-participants">会话成员</h2>
            <ul class="assist-members">
              <li v-for="device in devices" :key="device.role">
                <span class="assist-avatar" :class="device.role"><component :is="device.icon" :size="19" /></span>
                <div><b>{{ device.name }}</b><small>{{ device.label }}</small></div>
                <span v-if="role === device.role" class="assist-you">你</span>
              </li>
            </ul>
          </section>
          <section class="assist-card assist-details" aria-labelledby="assist-permissions">
            <h2 id="assist-permissions"><ShieldCheck :size="17" />协助方权限</h2>
            <dl class="assist-permissions">
              <div><dt><Eye :size="16" />观看画面</dt><dd :class="{ granted: joined }">{{ joined ? "已加入" : "未加入" }}</dd></div>
              <div>
                <dt><MousePointer2 :size="16" />鼠标和键盘</dt>
                <dd :class="{ granted: phase === 'controlling' }">{{ phase === "controlling" ? "已授权" : "未授权" }}</dd>
              </div>
            </dl>
            <p class="assist-permission-note">每次接管都由分享方确认，停止分享时会同时结束控制。</p>
          </section>
        </aside>
      </div>
    </main>

    <dialog ref="sourceDialog" class="assist-dialog" aria-labelledby="assist-dialog-title">
      <form @submit.prevent="startSharing">
        <header class="assist-dialog-heading">
          <div><h2 id="assist-dialog-title">选择要分享的画面</h2><p>将所选画面分享给{{ peer }}。</p></div>
          <button type="button" class="icon-button" aria-label="关闭" @click="sourceDialog?.close()"><X :size="20" /></button>
        </header>
        <fieldset class="assist-sources" aria-label="分享范围">
          <label v-for="option in sources" :key="option.id" class="assist-option">
            <input v-model="draftSource" type="radio" name="source" :value="option.id" />
            <div class="assist-option-preview" :class="option.id" aria-hidden="true">
              <component :is="option.icon" :size="37" :stroke-width="1.4" />
            </div>
            <div class="assist-option-info">
              <div><b>{{ option.label }}</b><small>{{ option.detail }}</small></div>
              <span class="assist-option-check" aria-hidden="true"><Check :size="13" :stroke-width="3" /></span>
            </div>
          </label>
        </fieldset>
        <footer class="assist-dialog-footer">
          <span><Eye :size="15" />对方加入后可观看画面</span>
          <div class="assist-actions">
            <button type="button" class="secondary" @click="sourceDialog?.close()">取消</button>
            <button type="submit" class="primary">开始分享</button>
          </div>
        </footer>
      </form>
    </dialog>
  </div>
</template>

<style scoped>
.assist-page {
  --assist-topbar-height: 66px;
  min-height: 100dvh;
  background: #f3f5f9;
  color: #253047;
}
.assist-topbar {
  position: sticky;
  top: 0;
  z-index: 2;
  border-bottom: 1px solid #e3e7ef;
  background: #fff;
}
.assist-topbar-inner,
.assist-workspace {
  width: min(100% - 72px, 1280px);
  margin-inline: auto;
}
.assist-topbar-inner,
.assist-brand,
.assist-perspective,
.assist-roles {
  display: flex;
  align-items: center;
}
.assist-topbar-inner { justify-content: space-between; min-height: var(--assist-topbar-height); gap: 12px; }
.assist-brand { gap: 10px; font-size: 16px; }
.assist-brand-mark {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 9px;
  background: #3964df;
  color: #fff;
  font-weight: 650;
}
.assist-prototype-label {
  margin-left: 5px;
  padding-left: 13px;
  border-left: 1px solid #e0e5ee;
  color: #68758a;
  font-size: 12px;
}
.assist-perspective { gap: 12px; color: #58667e; font-size: 12px; }
.assist-roles { gap: 2px; padding: 3px; border: 1px solid #e6eaf1; border-radius: 9px; background: #f1f3f7; }
.assist-roles button { padding: 6px 11px; border-radius: 6px; white-space: nowrap; }
.assist-roles button[aria-pressed="true"] {
  background: #fff;
  color: #2f55bf;
  font-weight: 650;
  box-shadow: 0 1px 3px #253b6722;
}
.assist-roles button:hover { color: #2f55bf; }
.assist-workspace { padding-block: 43px 36px; }
.assist-heading { margin-bottom: 23px; }
.assist-heading h1 { font-size: clamp(26px, 3.2vw, 35px); font-weight: 680; line-height: 1.25; }
.assist-heading p { margin-top: 7px; color: #626f83; font-size: 13px; }
.assist-layout { display: grid; grid-template-columns: minmax(0, 1fr) 278px; align-items: start; gap: 20px; }
.assist-main,
.assist-sidebar { display: grid; min-width: 0; gap: 16px; }
.assist-card { min-width: 0; border: 1px solid #e0e5ee; border-radius: 13px; background: #fff; }
.assist-screen { overflow: hidden; }
.assist-card-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 54px;
  padding: 8px 19px;
  border-bottom: 1px solid #e5e9f1;
}
.assist-card h2 { display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 650; }
.assist-card h2 svg { color: #546eac; }
.assist-source {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 9px;
  border-radius: 6px;
  background: #eef3fd;
  color: #3f5e9e;
  font-size: 11px;
}
.assist-source i { width: 6px; height: 6px; border-radius: 50%; background: #5980d7; }
.assist-stage { display: grid; place-items: center; padding: clamp(14px, 2.8vw, 32px); background: #e9eef7; }
.assist-empty {
  align-content: center;
  min-height: 445px;
  gap: 9px;
  background: radial-gradient(ellipse at 50% 41%, #ebf1ff, #f7f9fc 61%);
  text-align: center;
}
.assist-empty-icon {
  display: grid;
  place-items: center;
  width: 68px;
  height: 68px;
  margin-bottom: 12px;
  border: 1px solid #d6e2fc;
  border-radius: 19px;
  background: #e8efff;
  color: #4169cf;
  box-shadow: 0 0 0 9px #edf2fc, 0 0 0 10px #e3ebfa;
}
.assist-empty h3 { font-size: 21px; font-weight: 650; }
.assist-empty > p { color: #5e6c82; font-size: 13px; }
.assist-empty > button,
.assist-empty > .assist-actions { margin-top: 10px; }
.assist-page .primary,
.assist-page .secondary,
.assist-stop {
  min-height: 36px;
  padding: 7px 12px;
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
}
.assist-page .secondary { color: #43536d; }
.assist-stop { border: 1px solid #ebd6d7; border-radius: 7px; background: #fff; color: #a54047; }
.assist-stop:hover { background: #fff1f1; }
.assist-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 7px; }
.assist-banner {
  position: sticky;
  top: calc(var(--assist-topbar-height) + 9px);
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px 16px;
  padding: 13px 15px;
  border: 1px solid #d9e4fc;
  border-radius: 11px;
  background: #edf3fe;
  color: #31549a;
}
.assist-banner.needs-attention { border-color: #eddbb3; background: #fff8e7; color: #80531d; }
.assist-banner-message { display: flex; align-items: center; gap: 10px; }
.assist-banner-message p { margin-top: 3px; font-size: 12px; }
.assist-details { padding: 16px; }
.assist-members { display: grid; gap: 14px; margin: 16px 0 1px; padding: 0; list-style: none; }
.assist-members li { display: flex; align-items: center; gap: 10px; }
.assist-members b,
.assist-members small { display: block; }
.assist-members b { font-size: 13px; font-weight: 650; }
.assist-members small { margin-top: 2px; color: #68758a; font-size: 11px; }
.assist-avatar { display: grid; place-items: center; width: 37px; height: 37px; border-radius: 9px; }
.assist-avatar.sharer { background: #e8effe; color: #3a61bc; }
.assist-avatar.helper { background: #e9f3f1; color: #317765; }
.assist-you { margin-left: auto; padding: 2px 7px; border-radius: 5px; background: #f0f2f6; color: #58667d; font-size: 11px; }
.assist-permissions { display: grid; gap: 11px; margin: 15px 0 13px; }
.assist-permissions > div,
.assist-permissions dt { display: flex; align-items: center; gap: 7px; }
.assist-permissions > div { justify-content: space-between; }
.assist-permissions dt { color: #465570; font-size: 12px; }
.assist-permissions dt svg { color: #687ba1; }
.assist-permissions dd {
  margin: 0;
  padding: 3px 7px;
  border-radius: 5px;
  background: #f0f2f6;
  color: #58667d;
  font-size: 11px;
}
.assist-permissions dd.granted { background: #e6f4ee; color: #236952; }
.assist-permission-note { padding-top: 12px; border-top: 1px solid #ebedf3; color: #626f83; font-size: 12px; }
.assist-dialog {
  width: min(100% - 28px, 570px);
  max-height: calc(100dvh - 28px);
  padding: 0;
  border: 1px solid #e1e6ef;
  border-radius: 16px;
  background: #fff;
  color: #253047;
  box-shadow: 0 24px 66px #18274938;
}
.assist-dialog::backdrop { background: #18274366; }
.assist-dialog-heading,
.assist-dialog-footer {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 10px;
  padding: 16px 21px;
}
.assist-dialog-heading { padding-top: 21px; }
.assist-dialog-heading h2 { font-size: 19px; }
.assist-dialog-heading p { margin-top: 4px; color: #626f83; font-size: 13px; }
.assist-sources { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; min-width: 0; margin: 0; padding: 0 21px 19px; border: 0; }
.assist-option {
  position: relative;
  display: grid;
  gap: 10px;
  min-width: 0;
  padding: 8px;
  border: 1px solid #dce3ef;
  border-radius: 10px;
  color: #253047;
  cursor: pointer;
}
.assist-option:has(input:checked) { border-color: #5279e2; background: #f4f7ff; box-shadow: inset 0 0 0 1px #5279e2; }
.assist-option:has(input:focus-visible) { outline: 3px solid #a4b9fa; outline-offset: 2px; }
.assist-option input { position: absolute; inset: 0; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
.assist-option-preview { display: grid; place-items: center; min-height: 100px; border-radius: 6px; }
.assist-option-preview.desktop { background: linear-gradient(150deg, #294982, #9cb8e9); color: #fff; }
.assist-option-preview.window { background: #e9eef8; color: #4967a0; }
.assist-option-info { display: flex; justify-content: space-between; align-items: center; gap: 4px; padding: 0 3px 3px; }
.assist-option-info b { font-size: 13px; }
.assist-option-info small { display: block; color: #5e6c81; }
.assist-option-check { display: grid; place-items: center; flex-shrink: 0; width: 19px; height: 19px; border: 1px solid #c8d2e6; border-radius: 50%; background: #fff; color: transparent; }
.assist-option:has(input:checked) .assist-option-check { border-color: #3964df; background: #3964df; color: #fff; }
.assist-dialog-footer { align-items: center; border-top: 1px solid #e8ecf3; background: #fafbfe; }
.assist-dialog-footer > span { display: flex; align-items: center; gap: 7px; color: #626f83; font-size: 12px; }

@media (max-width: 920px) {
  .assist-topbar-inner, .assist-workspace { width: calc(100% - 48px); }
  .assist-layout { grid-template-columns: minmax(0, 1fr) 237px; gap: 14px; }
}
@media (max-width: 730px) {
  .assist-layout { grid-template-columns: minmax(0, 1fr); }
  .assist-sidebar { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .assist-workspace { padding-top: 26px; }
  .assist-empty { min-height: 330px; }
}
@media (max-width: 475px) {
  .assist-page { --assist-topbar-height: 55px; }
  .assist-topbar-inner, .assist-workspace { width: calc(100% - 28px); }
  .assist-brand { gap: 7px; font-size: 14px; }
  .assist-brand-mark { width: 27px; height: 27px; border-radius: 7px; }
  .assist-prototype-label { margin: 0; padding-left: 7px; font-size: 10px; }
  .assist-perspective > span { display: none; }
  .assist-roles button { gap: 3px; padding: 6px; }
  .assist-roles button svg { width: 13px; height: 13px; }
  .assist-heading { margin-bottom: 16px; }
  .assist-sidebar { grid-template-columns: minmax(0, 1fr); }
  .assist-dialog-heading, .assist-dialog-footer { padding-inline: 13px; }
  .assist-sources { gap: 8px; padding-inline: 13px; }
  .assist-dialog-footer { align-items: flex-end; flex-direction: column; }
}
</style>
