"""One-shot refresh receipts. This client cannot resolve an unknown server outcome."""
import fcntl,hashlib,os,time
from pathlib import Path

def refresh_once(api,out,save,read):
    from review_comparison import EditorialError, report_issues, run_root, unresolved
    out=Path(out);out.parent.mkdir(parents=True,exist_ok=True)
    checkpoint=out.with_suffix(out.suffix+'.checkpoint.json')
    lock=out.with_suffix(out.suffix+'.lock')
    with os.fdopen(os.open(lock,os.O_RDWR|os.O_CREAT,0o600),'w') as handle:
        fcntl.flock(handle,fcntl.LOCK_EX)
        if checkpoint.exists():
            receipt=read(checkpoint)
            if receipt.get('outcome') in ('completed','partial') and out.is_file():
                if hashlib.sha256(out.read_bytes()).hexdigest()!=receipt.get('result_file_sha256'):
                    raise EditorialError('refresh_receipt_changed','Saved refresh response changed after checkpoint','refresh')
                result=read(out)
                if receipt['outcome']=='partial':
                    raise EditorialError('refresh_partial','Actual refresh response reported feed failures; retain partial coverage','refresh')
                return result
            raise EditorialError('refresh_unreconciled','Refresh already started or failed; observe original evidence, do not replay it','refresh')
        blockers=[r for r in report_issues(run_root(out)) if r.get('stage') in ('refresh','audit') and unresolved([r])]
        if blockers:
            raise EditorialError('refresh_unreconciled','Existing run refresh/audit is unresolved; a new output path cannot replay it','refresh')
        if out.exists():
            raise EditorialError('refresh_receipt_missing','Existing refresh output has no bound invocation receipt; preserve it','refresh')
        receipt={'endpoint':'/api/agent/sources/refresh','started_at':int(time.time()),'outcome':'started','retry':False}
        save(checkpoint,receipt)
        try:
            result=api.call('/api/agent/sources/refresh',{},retry=False)
        except Exception as exc:
            receipt.update(outcome='rejected' if getattr(exc,'status',None) else 'unknown',
                           ended_at=int(time.time()),error_type=type(exc).__name__,http_status=getattr(exc,'status',None))
            save(checkpoint,receipt)
            raise
        # Keep the exact server report, including each feed error. No inferred success.
        save(out,result)
        valid=isinstance(result,dict) and isinstance(result.get('errors'),list) and isinstance(result.get('feeds_ok'),int) and isinstance(result.get('entries_cached'),int)
        receipt.update(outcome=('partial' if result['errors'] else 'completed') if valid else 'unverified_response',
                       ended_at=int(time.time()),result_file_sha256=hashlib.sha256(out.read_bytes()).hexdigest(),
                       feeds_ok=result.get('feeds_ok') if isinstance(result,dict) else None,
                       entries_cached=result.get('entries_cached') if isinstance(result,dict) else None,
                       error_count=len(result['errors']) if valid else None)
        save(checkpoint,receipt)
        if not valid:raise EditorialError('refresh_unverified_response','Actual response lacks the existing IngestReport fields; completion remains unverified','refresh')
        if result['errors']:raise EditorialError('refresh_partial','Actual refresh response reported feed failures; retain partial coverage','refresh')
        return result
