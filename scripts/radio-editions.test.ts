import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { buildRadioEditions } from '../frontend/src/radio-editions.ts';
import type { Item } from '../frontend/src/types.ts';
const make = (id: string, edition: string, audio: boolean, duration: number): Item => ({id, title:id, summary:null, original_url:null, cover_image_url:null, publish_time:1790006400, created_at:1790006400, tags:JSON.stringify(['radio:date:2026-09-21',`radio:edition:${edition}`]), audio_url:audio?`/audio/${id}.mp3`:null, duration_sec:duration});
test('logical editions survive late generation and count only playable duration',()=>{
  const groups=buildRadioEditions([make('m1','morning',true,61),make('m2','morning',false,900),make('e','evening',true,125)]);
  assert.equal(groups.length,2);
  const morning=groups.find(g=>g.key.endsWith('morning'))!;
  assert.match(morning.title,/2026年9月21日星期一/);
  assert.deepEqual(morning.playbackIds,['m1']);
  assert.equal(morning.totalDurationSec,61);
  const updated=buildRadioEditions([make('m1','morning',true,61),make('m2','morning',true,90)])[0];
  assert.equal(updated.playableCount,2); assert.equal(updated.totalDurationSec,151);
});
test('legacy timestamp uses Shanghai regardless of device timezone',()=>{
  const item={...make('x','morning',true,60),tags:null,publish_time:Date.parse('2026-09-20T23:30:00Z')/1000};
  assert.equal(buildRadioEditions([item])[0].key,'2026-09-21:morning');
});

test('a ready complete program replaces legacy tracks without double duration',()=>{
  const old=make('old','morning',true,61);
  const program={...make('program','morning',true,600),tags:JSON.stringify(['radio:date:2026-09-21','radio:edition:morning','radio:program','radio:sections:11'])};
  const group=buildRadioEditions([old,program])[0];
  assert.deepEqual(group.playbackIds,['program']);assert.equal(group.totalDurationSec,600);
  assert.deepEqual(buildRadioEditions([old,{...program,audio_url:null}])[0].playbackIds,['old']);
});
