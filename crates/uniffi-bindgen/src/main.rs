// Generates the Kotlin bindings: `bindings src:nori-android <out dir>`.
fn main() {
    uniffi_bindgen_kotlin_jni::main().unwrap();
    let args: Vec<String> = std::env::args().collect();
    if let (Some("bindings"), Some(out)) = (args.get(1).map(String::as_str), args.get(3)) {
        let path = std::path::Path::new(out).join("uniffi").join("Uniffi.kt");
        if let Ok(kt) = std::fs::read_to_string(&path) {
            std::fs::write(&path, cancellable(&nullable_futures(&kt))).unwrap();
        }
    }
}

/// Upstream bug workaround: async exports end in `return completion.value!!`, which throws when a
/// nullable return (`T?`) is null. Drops the `!!` for those. Remove once the pinned revision fixes it.
fn nullable_futures(kt: &str) -> String {
    let mut out = String::with_capacity(kt.len());
    let mut nullable = false;
    for line in kt.split_inclusive('\n') {
        if let Some(ret) = line.trim_end().strip_prefix(") : ") {
            nullable = ret.ends_with('?');
        }
        if nullable && line.trim() == "return completion.value!!" {
            out.push_str(&line.replacen("completion.value!!", "completion.value", 1));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// NORI: upstream's await does not hear the coroutine's cancellation, so a flow that let go of a call
/// waited for all of it (`flatMapLatest` joins the call it cancels). Awaits are made cancellable and tell
/// the Rust future, which is dropped with its handle (crates/android/build.rs). Async Kotlin calls Rust
/// makes are kept by their oneshot handle, so a Rust future dropped mid-call cancels them.
fn cancellable(kt: &str) -> String {
    let mut out = String::with_capacity(kt.len());
    let mut future = None;
    let mut call = false;
    for line in kt.split_inclusive('\n') {
        if let Some(id) = line.strip_prefix("suspend fun awaitRustFuture").and_then(|l| l.strip_suffix("(\n")) {
            future = Some(id.to_string());
        }
        if line.trim() == "val futureReady = kotlin.coroutines.suspendCoroutine<kotlin.Boolean> { continuation ->" {
            let id = future.as_deref().expect("an await is inside its awaitRustFuture function");
            out.push_str(&line.replacen("kotlin.coroutines.suspendCoroutine", "kotlinx.coroutines.suspendCancellableCoroutine", 1));
            out.push_str(&format!("                continuation.invokeOnCancellation {{ Scaffolding.rustFutureCancel{id}(rustFuture) }}\n"));
        } else if line.trim() == "kotlinx.coroutines.GlobalScope.launch uniffiCoroutineBlock@ {" {
            call = true;
            out.push_str(&line.replacen(".launch ", ".launch(start = kotlinx.coroutines.CoroutineStart.LAZY) ", 1));
        } else if call && line == "    }\n" {
            call = false;
            out.push_str("    }.let { uniffiKotlinCalls.start(uniffiOneshotHandle, it) }\n");
        } else {
            out.push_str(line);
        }
    }
    out.push_str(KOTLIN_CALLS);
    out
}

const KOTLIN_CALLS: &str = "
/** NORI: the async Kotlin calls Rust waits for, by oneshot handle (held by Rust while it waits, so never reused). */
internal object uniffiKotlinCalls {
    private val jobs = java.util.concurrent.ConcurrentHashMap<kotlin.Long, kotlinx.coroutines.Job>()

    fun start(handle: kotlin.Long, job: kotlinx.coroutines.Job) {
        jobs[handle] = job
        job.invokeOnCompletion { jobs.remove(handle, job) }
        job.start()
    }

    fun cancel(handle: kotlin.Long) {
        jobs.remove(handle)?.cancel()
    }
}

/** NORI: Rust stopped waiting for the call under [handle]. */
fun uniffiKotlinCallDropped(handle: kotlin.Long) = uniffiKotlinCalls.cancel(handle)
";

#[cfg(test)]
mod tests {
    #[test]
    fn nullable_future_drops_non_null_assertion() {
        let kt = "suspend fun a(\n    f: kotlin.Long,\n) : kotlin.String?\n{\n                return completion.value!!\n}\nsuspend fun b(\n    f: kotlin.Long,\n) : kotlin.String\n{\n                return completion.value!!\n}\n";
        let fixed = super::nullable_futures(kt);
        assert_eq!(fixed.matches("return completion.value!!").count(), 1);
        assert!(fixed.contains("kotlin.String?\n{\n                return completion.value\n}"));
    }

    #[test]
    fn awaits_and_kotlin_calls_cancel() {
        let kt = "suspend fun awaitRustFuture7(\n    rustFuture: kotlin.Long,\n)\n{\n            val futureReady = kotlin.coroutines.suspendCoroutine<kotlin.Boolean> { continuation ->\n            }\n}\nfun dispatch(\n) {\n    kotlinx.coroutines.GlobalScope.launch uniffiCoroutineBlock@ {\n        try {\n        }\n    }\n}\n";
        let fixed = super::cancellable(kt);
        assert!(fixed.contains("kotlinx.coroutines.suspendCancellableCoroutine<kotlin.Boolean> { continuation ->\n                continuation.invokeOnCancellation { Scaffolding.rustFutureCancel7(rustFuture) }\n"));
        assert!(fixed.contains("GlobalScope.launch(start = kotlinx.coroutines.CoroutineStart.LAZY) uniffiCoroutineBlock@ {\n        try {\n        }\n    }.let { uniffiKotlinCalls.start(uniffiOneshotHandle, it) }\n}\n"));
    }
}
