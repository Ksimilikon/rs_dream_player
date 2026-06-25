use rodio::Source;

pub struct SourceCallback<S, F>
where
    S: Source,
    F: FnOnce() + Send + 'static,
{
    inner: Box<S>,
    callback: Option<F>,
}
impl<S, F> SourceCallback<S, F>
where
    S: Source,
    F: FnOnce() + Send + 'static,
{
    pub fn new(src: Box<S>, f: F) -> Self {
        Self {
            inner: src,
            callback: Some(f),
        }
    }
}

impl<S, F> Iterator for SourceCallback<S, F>
where
    S: Source,
    F: FnOnce() + Send + 'static,
{
    type Item = S::Item;
    fn next(&mut self) -> Option<Self::Item> {
        let next_sample = self.inner.next();
        if next_sample.is_none()
            && let Some(cb) = self.callback.take()
        {
            cb();
        }
        next_sample
    }
}

impl<S, F> Source for SourceCallback<S, F>
where
    S: Source,
    F: FnOnce() + Send + 'static,
    S::Item: rodio::cpal::Sample,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}
