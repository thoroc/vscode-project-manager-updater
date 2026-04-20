macro_rules! log_info {
    ($($arg:tt)*) => {
        eprintln!(
            "[{}] {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            format_args!($($arg)*)
        )
    };
}

pub(crate) use log_info;
