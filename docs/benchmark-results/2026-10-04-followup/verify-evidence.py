import collections, datetime, hashlib, json, math
from pathlib import Path
R = Path('/home/dev/workspace/projects/rusta-pitest/docs/benchmark-results/sleepapi-followup-2026-10-04-01a0ef83')
S = Path('/home/dev/workspace/projects/sleepapi/docs/benchmark-results/2026-10-04-followup')
def read(p): return json.loads(p.read_text())
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
setup=read(S/'docker-setup.json')
containers={'/sleep/'+x['role']:x for x in setup['containers'] if x['role']!='recovery'}
receipts=sorted((R/'execution').glob('*/receipt.json'))
assert len(receipts)==48, len(receipts)
seen=set(); rows=[]; hashed=0; all_samples=0
for path in receipts:
    r=read(path); d=path.parent
    assert r['execution_state']=='completed' and r['sampler_exit_code']==0
    key=(r['route'],r['concurrency'],r['repeat']); assert key not in seen; seen.add(key)
    c=containers[r['route']]
    assert r['container']['container_id']==c['container_id']
    for name,digest in r['artifact_sha256'].items():
        assert sha(d/name)==digest, str(d/name)
        hashed+=1
    doc=read(d/'report.json'); report=doc['runs'][0]['report']; m=report['metrics']
    assert doc['passed'] and doc['runner']['build_profile']=='release'
    assert report['request']['url']==c['base_url']+r['route']
    assert report['workload']['effective_concurrency']==r['concurrency']
    assert report['workload']['duration_secs']==5
    assert m['attempted']==m['succeeded']==m['complete_responses']==m['latency']['samples']
    assert m['status_counts']=={'200':m['attempted']}
    assert all(m[k]==0 for k in ('timeouts','transport_errors','unexpected_status'))
    assert math.isclose(m['successful_responses_per_sec'],m['succeeded']/m['elapsed_secs'])
    assert 'contract_validation' not in report
    raw=[json.loads(line) for line in (d/'sampler.jsonl').read_text().splitlines()]
    metadata=raw[0]; samples=[x for x in raw if x['type']=='sample']; summary=raw[-1]
    assert metadata['container_id']==c['container_id'] and metadata['pid']==c['pid']
    assert metadata['image_id']==c['image_id'] and metadata['scope']=='container_init_process'
    assert summary['type']=='summary' and summary['reason']=='stop_file'
    assert summary['sample_count']==len(samples)
    assert all(a['monotonic_ns']<b['monotonic_ns'] for a,b in zip(samples,samples[1:]))
    res=read(d/'resources.json'); all_samples+=len(samples)
    assert r['phases']['idle']['end']['monotonic_ns']<=r['phases']['warmup']['start']['monotonic_ns']
    assert r['phases']['warmup']['end']['monotonic_ns']<=r['phases']['measured']['start']['monotonic_ns']
    for phase in ('idle','warmup','measured'):
        bounds=r['phases'][phase]; agg=res['phases'][phase]
        selected=[x for x in samples if bounds['start']['monotonic_ns']<=x['monotonic_ns']<=bounds['end']['monotonic_ns']]
        assert len(selected)==agg['sample_count'] and len(selected)>=2
        first,last=selected[0],selected[-1]
        span=(last['monotonic_ns']-first['monotonic_ns'])/1e9
        ticks=(last['user_cpu_ticks']+last['system_cpu_ticks']-first['user_cpu_ticks']-first['system_cpu_ticks'])
        cpu=ticks/metadata['clock_ticks_per_second']/span*100
        assert math.isclose(cpu,agg['cpu_percent_one_core'],abs_tol=1e-12)
        assert agg['rss_bytes']['max']==max(x['rss_bytes'] for x in selected)
        assert agg['live_threads']['max']==max(x['live_threads'] for x in selected)
    measured=res['phases']['measured']
    rows.append({'sequence':r['sequence'],'route':r['route'],'concurrency':r['concurrency'],'repeat':r['repeat'],
                 'attempted':m['attempted'],'rps':m['successful_responses_per_sec'],'p50_ms':m['latency']['p50_us']/1000,
                 'p99_ms':m['latency']['p99_us']/1000,'cpu_percent_one_core':measured['cpu_percent_one_core'],
                 'peak_rss_bytes':measured['rss_bytes']['max'],'peak_live_threads':measured['live_threads']['max'],
                 'sample_count':measured['sample_count'],'max_sample_gap_secs':measured['sample_gap_summary_secs']['max']})
assert seen=={(route,c,rep) for route in containers for c in (1,4,16,64) for rep in (1,2,3)}
contract=R/'contract-recovery'; cr=read(contract/'execution-summary.json')
assert cr['execution_state']=='completed' and len(cr['completed_profiles'])==15 and len(cr['completed_recovery_routes'])==4
checked=0
for name in cr['completed_profiles']:
    doc=read(contract/name/'report.json'); assert doc['passed']
    assert len(doc['runs'])==4
    for run in doc['runs']:
        v=run['report']['contract_validation']; assert v['checked']==v['passed']==1 and v['failed']==0
        checked+=1
recovery=[]
for entry in cr['completed_recovery_routes']:
    receipt=read(contract/entry['receipt']); assert receipt['execution_state']=='completed' and receipt['exit_code']==1
    m=receipt['native_timeout_report']['metrics']; assert m['attempted']==m['timeouts']==1 and m['complete_responses']==0
    probes=[json.loads(line) for line in (contract/entry['receipt']).with_name('probes.jsonl').read_text().splitlines()]
    occupied=[x for x in probes if x['phase']=='establish_capacity' and x.get('status')==429]
    assert occupied and occupied[-1]['body_json']['simulated'] is False
    assert occupied[-1]['body_json']['error']['code']=='capacity_exceeded'
    assert occupied[-1]['finished_monotonic_secs']<receipt['timeout_exit_observation_monotonic_bounds_secs'][1]
    admitted=probes[-1]; assert admitted['phase']=='after_timeout' and admitted['status']==200
    b=admitted['body_json']; assert b['simulated'] is True and b['requested_duration_ms']==0 and b['operation']=='sleep'
    recovery.append({'route':entry['route'],'after_launch_secs':receipt['recovery_secs_after_launch_bounds'],
                     'after_timeout_observed_secs':receipt['recovery_secs_after_timeout_observed_bounds'],
                     'probe_count':len(probes)})
result={'operation':setup['operation'],'verified_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'passed':True,'verified_runs':48,'measured_requests':sum(x['attempted'] for x in rows),
        'contract_responses_checked':checked,'intentional_timeouts':4,'recovered_routes':4,
        'verified_artifact_hashes':hashed,'resource_samples':all_samples,'rows':rows,'recovery':recovery,
        'checks':['48 unique route/concurrency/repetition combinations','saved artifact SHA256 values',
                  'native HTTP status accounting and rates','route/container/image/PID identity',
                  'separate idle/warmup/measured intervals','independent CPU delta/RSS/thread recomputation',
                  'all 60 strict contract responses passed','four true-capacity probes, intentional timeouts, valid simulation recovery']}
with (S/'independent-verification.json').open('x') as f:json.dump(result,f,indent=2);f.write('\n')
print(json.dumps({k:v for k,v in result.items() if k not in ('rows','checks')},indent=2))
