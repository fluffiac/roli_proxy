use std::sync::Arc;

use crate::api;

pub type Query = Arc<str>;

pub struct QueryBuilder(String);

impl QueryBuilder {
    pub const fn new() -> Self {
        Self(String::new())
    }

    pub fn into_query(self) -> Query {
        Arc::from(self.0.into_boxed_str())
    }

    pub fn push_header(&mut self, reget: usize, previews: usize, refresh: usize) -> &mut Self {
        self.push_element("600000")
            .push_element(&reget.to_string())
            .push_element(&previews.to_string())
            .push_element(&refresh.to_string())
    }

    pub fn push_post(&mut self, post: &api::Post, ids: (usize, usize)) -> &mut Self {
        self.push_newline()
            .push_element(&ids.0.to_string())
            .push_element(&post.id.to_string())
            .push_element(&post.file.width.to_string())
            .push_element(&post.file.height.to_string())
            .push_element(&post.preview.width.to_string())
            .push_element(&post.preview.height.to_string())
            .push_element(&post.score.up.to_string())
            .push_element(&post.score.down.to_string())
            .push_element(&post.rating)
            .push_element(&post.file.ext)
            .push_element(&ids.1.to_string())
            .push_element("1200000")
    }

    pub fn push_element(&mut self, element: &str) -> &mut Self {
        if let None | Some('\n') = self.0.chars().last() {
        } else {
            self.0.push(',');
        }
        self.0.push_str(element);
        self
    }

    pub fn push_newline(&mut self) -> &mut Self {
        self.0.push('\n');
        self
    }
}
