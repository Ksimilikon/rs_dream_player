use std::any::Any;

pub trait DBus {
    fn send_metadata(&self, title: String, artists: Vec<String>, art: Option<Vec<u8>>);
    fn play(&self, callback: &mut dyn FnMut(&mut dyn Any));
    fn pause(&self, callback: &mut dyn FnMut(&mut dyn Any));
    fn play_pause(&self, callback: &mut dyn FnMut(&mut dyn Any));
    fn stop(&self, callback: &mut dyn FnMut(&mut dyn Any));
    fn next(&self, callback: &mut dyn FnMut(&mut dyn Any));
    fn prev(&self, callback: &mut dyn FnMut(&mut dyn Any));
}
