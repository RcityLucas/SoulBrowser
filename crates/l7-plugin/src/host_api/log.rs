use super::{HostApi, HostContext, LogLevel, NoopHostApi};

pub trait LogApi {
    fn log(&self, ctx: &HostContext, level: LogLevel, message: &str);
}

impl<T> LogApi for T
where
    T: HostApi + ?Sized,
{
    fn log(&self, ctx: &HostContext, level: LogLevel, message: &str) {
        HostApi::log(self, ctx, level, message)
    }
}

pub type DefaultLogApi = NoopHostApi;
