import { nextTick, onUnmounted, watch, type Ref } from 'vue';

/** Keep keyboard focus inside the active modal and restore its opener on close. */
export function useModalFocus(
  active: Ref<string>,
  container: Ref<HTMLElement | undefined>,
  close: () => void,
) {
  let opener: HTMLElement | null = null;
  const focusable = () => Array.from(container.value?.querySelectorAll<HTMLElement>(
    'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]',
  ) ?? []).filter(element => element.getClientRects().length > 0);

  function onKeydown(event: KeyboardEvent) {
    if (!active.value) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    } else if (event.key === 'Tab') {
      const items = focusable();
      const first = items[0];
      const last = items[items.length - 1];
      if (!first) {
        event.preventDefault();
        container.value?.focus();
      } else if (event.shiftKey && (document.activeElement === first || !container.value?.contains(document.activeElement))) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && (document.activeElement === last || !container.value?.contains(document.activeElement))) {
        event.preventDefault();
        first.focus();
      }
    }
  }

  watch(active, async (value, previous) => {
    if (value && !previous) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      document.addEventListener('keydown', onKeydown);
    }
    await nextTick();
    if (value !== active.value) return;
    if (value) {
      (container.value?.querySelector<HTMLElement>('input:not(:disabled)') ?? focusable()[0] ?? container.value)?.focus();
    } else {
      document.removeEventListener('keydown', onKeydown);
      if (opener?.isConnected) opener.focus();
      opener = null;
    }
  });
  onUnmounted(() => document.removeEventListener('keydown', onKeydown));
}
