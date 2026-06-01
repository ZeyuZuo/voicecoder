import { invoke } from "@tauri-apps/api/core";
import type { VoiceAudioChunkPayload } from "../types/app";

const TARGET_SAMPLE_RATE = 16000;
const CHUNK_SIZE_BYTES = 6400;

export type VoiceCaptureController = {
  stop: () => void;
};

type CreateVoiceCaptureOptions = {
  sessionId: string;
  onError: (message: string) => void;
};

export async function requestVoiceInputStream(): Promise<MediaStream> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error("当前运行环境不支持麦克风采集。");
  }

  return navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: false,
      noiseSuppression: false,
      autoGainControl: false
    }
  });
}

export async function createVoiceCapture(stream: MediaStream, { sessionId, onError }: CreateVoiceCaptureOptions): Promise<VoiceCaptureController> {
  const AudioContextClass = window.AudioContext ?? window.webkitAudioContext;
  if (!AudioContextClass) {
    stopMediaStream(stream);
    throw new Error("当前运行环境不支持 AudioContext。");
  }

  const audioContext = new AudioContextClass();
  const source = audioContext.createMediaStreamSource(stream);
  const processor = audioContext.createScriptProcessor(4096, 1, 1);
  const pcmBuffer: number[] = [];
  let sequence = 0;
  let stopped = false;

  const sendAudioChunk = (data: number[]) => {
    const payload: VoiceAudioChunkPayload = {
      sessionId,
      sampleRate: TARGET_SAMPLE_RATE,
      channels: 1,
      format: "pcm_s16le",
      sequence,
      data
    };

    sequence += 1;
    void invoke("send_voice_audio_chunk", { chunk: payload }).catch((error) => {
      stop();
      onError(error instanceof Error ? error.message : String(error));
    });
  };

  const stop = () => {
    if (stopped) {
      return;
    }

    stopped = true;
    if (pcmBuffer.length > 0) {
      sendAudioChunk(pcmBuffer.splice(0, pcmBuffer.length));
    }
    processor.disconnect();
    source.disconnect();
    stopMediaStream(stream);
    void audioContext.close();
  };

  processor.onaudioprocess = (event) => {
    event.outputBuffer.getChannelData(0).fill(0);

    if (stopped) {
      return;
    }

    const input = event.inputBuffer.getChannelData(0);
    const resampled = resampleTo16k(input, audioContext.sampleRate);
    const pcm = floatToPcm16(resampled);

    for (const byte of pcm) {
      pcmBuffer.push(byte);
    }

    while (pcmBuffer.length >= CHUNK_SIZE_BYTES) {
      const data = pcmBuffer.splice(0, CHUNK_SIZE_BYTES);
      sendAudioChunk(data);
    }
  };

  source.connect(processor);
  processor.connect(audioContext.destination);

  return {
    stop
  };
}

function resampleTo16k(input: Float32Array, sourceSampleRate: number): Float32Array {
  if (sourceSampleRate === TARGET_SAMPLE_RATE) {
    return input;
  }

  const ratio = sourceSampleRate / TARGET_SAMPLE_RATE;
  const outputLength = Math.max(1, Math.round(input.length / ratio));
  const output = new Float32Array(outputLength);

  for (let index = 0; index < outputLength; index += 1) {
    const sourceIndex = index * ratio;
    const before = Math.floor(sourceIndex);
    const after = Math.min(before + 1, input.length - 1);
    const weight = sourceIndex - before;
    output[index] = input[before] * (1 - weight) + input[after] * weight;
  }

  return output;
}

function floatToPcm16(input: Float32Array): number[] {
  const output = new Uint8Array(input.length * 2);
  const view = new DataView(output.buffer);

  for (let index = 0; index < input.length; index += 1) {
    const sample = Math.max(-1, Math.min(1, input[index]));
    const value = sample < 0 ? sample * 0x8000 : sample * 0x7fff;
    view.setInt16(index * 2, value, true);
  }

  return Array.from(output);
}

function stopMediaStream(stream: MediaStream) {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}

declare global {
  interface Window {
    webkitAudioContext?: typeof AudioContext;
  }
}
