import { test, expect } from '@playwright/test';
import { RequestTimeoutError, withDeadline } from '../src/request';

test('request deadline aborts stalled work and reports a distinct timeout', async () => {
 let signal: AbortSignal | undefined;
 await expect(withDeadline(s => {
  signal = s;
  return new Promise(() => {});
 }, 20)).rejects.toBeInstanceOf(RequestTimeoutError);
 expect(signal?.aborted).toBe(true);
});

test('deadline remains active while reading a response body after headers', async () => {
 let signal: AbortSignal | undefined;
 const response = new Response(new ReadableStream());
 await expect(withDeadline(async s => {
  signal = s;
  // Headers have arrived, but the JSON stream never completes.
  return await response.json();
 }, 20)).rejects.toBeInstanceOf(RequestTimeoutError);
 expect(signal?.aborted).toBe(true);
 await response.body?.cancel().catch(() => {});
});

test('successful and failed operations clear their timers without changing errors', async () => {
 let signal: AbortSignal | undefined;
 expect(await withDeadline(async s => { signal = s; return 42; }, 20)).toBe(42);
 const failure = new Error('original failure');
 await expect(withDeadline(async () => { throw failure; }, 20)).rejects.toBe(failure);
 await new Promise(resolve => setTimeout(resolve, 40));
 expect(signal?.aborted).toBe(false);
});
