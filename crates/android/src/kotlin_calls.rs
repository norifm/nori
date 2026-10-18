// NORI: included into the generated scaffolding (build.rs `cancellable_calls`).

/// An async Kotlin call (a Transport request) Rust waits for. Dropped before the answer, it has Kotlin
/// cancel the call's coroutine, and with it the OkHttp call. The receiver keeps the oneshot, and so
/// its handle, from being reused until Kotlin has been told.
struct UniffiKotlinCall<T> {
    oneshot: i64,
    answer: uniffi::oneshot::Receiver<T>,
    answered: bool,
}

impl<T> ::std::future::Future for UniffiKotlinCall<T> {
    type Output = T;

    fn poll(mut self: ::std::pin::Pin<&mut Self>, cx: &mut ::std::task::Context<'_>) -> ::std::task::Poll<T> {
        let answer = ::std::pin::Pin::new(&mut self.answer).poll(cx);
        self.answered = answer.is_ready();
        answer
    }
}

impl<T> Drop for UniffiKotlinCall<T> {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        static DROPPED: uniffi_jni::CachedStaticMethod = uniffi_jni::CachedStaticMethod::new(
            c"uniffi/UniffiKt",
            c"uniffiKotlinCallDropped",
            c"(J)V",
        );
        // Safety: uniffi_get_global_jvm() is the loaded JVM and the argument matches the signature.
        unsafe {
            uniffi_jni::attach_current_thread(uniffi_get_global_jvm(), |env| {
                if DROPPED.call_void(env, [uniffi_jni::jvalue { j: self.oneshot }]).is_err() {
                    ((**env).v1_2.ExceptionClear)(env);
                }
            });
        }
    }
}
