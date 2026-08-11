import {
  decodeWzCanvas,
  extractWzSound,
  extractWzVideo,
  decodeMsCanvas,
  extractMsSound,
  extractMsVideo,
} from '../../ts-wrapper/wasm-pkg/wzlib_rs.js';
import { state } from './state.js';

export function dispatchDecodeCanvas(imgOffsetOrEntryIndex, propPath) {
  if (state.fileMode === 'ms') {
    return decodeMsCanvas(state.wzData, state.msFileName, imgOffsetOrEntryIndex, propPath);
  }
  return decodeWzCanvas(state.wzData, state.wzVersionName, imgOffsetOrEntryIndex, state.wzVersionHash, propPath);
}

export function dispatchExtractSound(imgOffsetOrEntryIndex, propPath) {
  if (state.fileMode === 'ms') {
    return extractMsSound(state.wzData, state.msFileName, imgOffsetOrEntryIndex, propPath);
  }
  return extractWzSound(state.wzData, state.wzVersionName, imgOffsetOrEntryIndex, state.wzVersionHash, propPath);
}

export function dispatchExtractVideo(imgOffsetOrEntryIndex, propPath) {
  if (state.fileMode === 'ms') {
    return extractMsVideo(state.wzData, state.msFileName, imgOffsetOrEntryIndex, propPath);
  }
  return extractWzVideo(state.wzData, state.wzVersionName, imgOffsetOrEntryIndex, state.wzVersionHash, propPath);
}
