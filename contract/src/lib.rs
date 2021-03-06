/*
 * Decentrablog
 *
 * Learn more about writing NEAR smart contracts with Rust:
 * https://github.com/near/near-sdk-rs
 *
 */

// To conserve gas, efficient serialization is achieved through Borsh (http://borsh.io/)
use std::convert::TryInto;
use comment::Comment;
use donation::DonationLog;
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::{env, near_bindgen, setup_alloc, AccountId, Promise};
use near_sdk::collections::{UnorderedMap};
use near_sdk::serde::{Serialize, Deserialize};
use post::Post;

setup_alloc!();

type PostId = usize;
type CommentId = usize;

mod comment;
mod post;
mod donation;

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
pub struct Blog {
    owner: AccountId,
    user_posts: UnorderedMap<AccountId, Vec<usize>>,
    posts: UnorderedMap<PostId, Post>,
    comments: UnorderedMap<CommentId, Comment>,

    next_post_id: usize,
    next_comment_id: usize,
    next_donation_id: usize,
}

#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
#[serde(crate = "near_sdk::serde")]
pub enum VoteStatus {
    Upvoted,
    Downvoted,
    None,
}

impl Default for Blog {
  fn default() -> Self {
    Self {
      owner: env::signer_account_id(),
      user_posts: UnorderedMap::new(b"user_posts".to_vec()),
      posts: UnorderedMap::new(b"posts".to_vec()),
      comments: UnorderedMap::new(b"comments".to_vec()),

      next_post_id: 0,
      next_comment_id: 0,
      next_donation_id: 0,
    }
  }
}

#[near_bindgen]
impl Blog {
    pub fn create_post(&mut self, title: String, body: String) -> usize {
        let post_id = self.next_post_id;

        let post =  Post::new(post_id, title, body, env::predecessor_account_id(), env::block_timestamp());
        
        self.posts.insert(&post_id, &post);
        self.next_post_id = self.next_post_id + 1;

        //push to user's post list
        let mut user_posts = self.user_posts.get(&env::predecessor_account_id()).unwrap_or(vec![]);
        
        user_posts.push(post_id);
        self.user_posts.insert(&env::predecessor_account_id(), &user_posts); 

        let title = post.get_title();

        // Use env::log to record logs permanently to the blockchain!
        env::log(format!("Post '{}' was created", title).as_bytes());

        post_id
    }

    pub fn get_owner(&self) -> AccountId {
        self.owner.clone()
    }

    pub fn get_post(&self, post_id: usize) -> Option<Post> {
        self.posts.get(&post_id)
    }

    pub fn get_posts(&self) -> Vec<Post> {
        let mut posts = Vec::new();

        for post_id in self.posts.keys() {
            posts.push(self.posts.get(&post_id).unwrap());
        }

        posts
    }

    pub fn get_user_posts(&self, user_id: AccountId) -> Vec<Post> {
        //if user_id has no post by checking length
        if self.user_posts.get(&user_id).unwrap_or(vec![]).len() == 0 {
            return vec![];
        }

        let mut posts = Vec::new();

        for post_id in self.user_posts.get(&user_id).unwrap() {
            posts.push(self.posts.get(&post_id).unwrap());
        }

        posts
    }

    pub fn get_paging_posts(&self, page: usize, page_size: usize) -> Vec<Post> {
        assert!(page_size > 0, "Page size must be greater than 0");
        assert!(page > 0, "Page must be greater than 0");

        let mut posts = Vec::new();


        //notice: page start from 1
        let start = (page - 1) * page_size;
        let mut end = start + page_size;
    
        let total_posts = self.posts.len().try_into().unwrap_or(0);

        if end > total_posts {
            end = total_posts;
        }

        let values = self.posts.values();

        for value in values.skip(start).take(end - start) {
            posts.push(value);
        }

        posts
    }

    pub fn get_total_posts(&self) -> u64 {
        self.posts.len()
    }

    pub fn delete_post(&mut self, post_id: usize) {
        assert_eq!(self.owner, env::predecessor_account_id(), "Only owner can delete posts");
        self.posts.remove(&post_id);
    }

    pub fn create_comment(&mut self, post_id: usize, body: String) {
        // Check if the post exists
        let post = self.posts.get(&post_id);
        assert!(post.is_some(), "Post does not exist");
        assert!(body.len() >= 10, "Comment must be at least 10 characters long");

        let author = env::predecessor_account_id();
        let created_at = env::block_timestamp();

        let comment = Comment::new(self.next_comment_id, body, author, created_at);

        match self.posts.get(&post_id).as_mut() {
            Some(post) => {
                post.add_comment(comment.get_comment_id());
                self.posts.insert(&post_id, post);
            },
            None => panic!("Post does not exist"),
        }

        self.comments.insert(&comment.get_comment_id(), &comment);
        self.next_comment_id = self.next_comment_id + 1;
    }

    pub fn delete_comment(&mut self, post_id: usize, comment_id: usize) {
        // only owner can delete comments
        assert_eq!(self.owner, env::predecessor_account_id(), "Only owner can delete comments");

        // Check if the post exists
        let mut post = match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };
        
        let comment = self.comments.get(&comment_id).unwrap();
        assert!(comment.get_comment_id() == comment_id, "Comment does not exist");
        
        post.remove_comment(comment_id);
    }

    #[payable]
    pub fn donate(&mut self, post_id: usize, amount: u128, message: String) {
        // Check if the post exists
        let post = self.posts.get(&post_id).unwrap();
        assert!(post.get_post_id() == post_id, "Post does not exist");

        // check if the amount is valid
        assert!(amount >= 1, "Amount must be greater than 0");
        // enough balance
        assert!(env::account_balance() >= amount, "Not enough balance");


        // transfer NEAR to the post author
        let author = post.get_author();
        let amount = amount;
        
        Promise::new(author).transfer(amount).then(self.save_to_donation_log(post_id, amount, message));
    }

    #[private]
    fn save_to_donation_log(&mut self, post_id: usize, amount: u128, message: String) -> Promise {
        let created_at = env::block_timestamp();

        let donation_log = DonationLog::new(self.next_donation_id, amount, env::predecessor_account_id(), created_at, message, post_id);

        self.next_comment_id = self.next_comment_id + 1;

        // save to donation log
        let mut post = self.posts.get(&post_id).unwrap();
        post.add_donation_logs(donation_log);
        self.posts.insert(&post_id, &post);

        let donor = env::predecessor_account_id();

        //Mark the promise as fulfilled by doing nothing
        Promise::new(donor)
    }

    pub fn get_next_post_id(&self) -> usize {
        self.next_post_id
    }

    pub fn get_comments(&self, post_id: usize) -> Vec<Comment> {
        let post = self.posts.get(&post_id).unwrap();
        
        let mut comments = Vec::new();
        for comment_id in post.get_comments() {
            comments.push(self.comments.get(&comment_id).unwrap());
        }
        comments
    }

    pub fn get_paging_comments(self, post_id: usize, page: usize, page_size: usize) -> Vec<Comment> {
        assert!(page_size > 0, "Page size must be greater than 0");
        assert!(page > 0, "Page must be greater than 0");

        let post = self.posts.get(&post_id).unwrap();
        let mut comments = Vec::new();

        let mut start = (page - 1) * page_size;
        let mut end = start + page_size;
         
        if end > post.get_comments().len() {
            end = post.get_comments().len();
        }

        for comment_id in post.get_comments() {
            if start < end {
                start = start + 1;
                comments.push(self.comments.get(&comment_id).unwrap());
            }
        }

        comments
    }

    pub fn get_total_comments(&self) -> u64 {
        self.comments.len()
    }

    pub fn get_comment(&self, comment_id: usize) -> Comment {
        self.comments.get(&comment_id).unwrap()
    }

    pub fn get_post_total_comments(&self, post_id: usize) -> u64 {
        let post = match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };
        
        post.get_comments().len().try_into().unwrap()
    }

    pub fn upvote(&mut self, post_id: usize) {
        let voter = env::predecessor_account_id(); 
        
        
        match self.posts.get(&post_id).as_mut() {
            Some(post) => {
                post.add_upvote(voter);
                self.posts.insert(&post_id, post);
            },
            None => panic!("Post does not exist"),
        }
    }

    pub fn remove_upvote(&mut self, post_id: usize) {
        let mut post = match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };

        let voter = env::predecessor_account_id(); 
        post.remove_upvote(voter);
        
        self.posts.insert(&post_id, &post);
    }

    pub fn downvote(&mut self, post_id: usize) {
        let voter = env::predecessor_account_id(); 

        match self.posts.get(&post_id).as_mut() {
            Some(post) => {
                post.add_downvote(voter);
                self.posts.insert(&post_id, post);
            },
            None => panic!("Post does not exist"),
        }
    }

    pub fn remove_downvote(&mut self, post_id: usize) {
        let mut post =  match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };        
        let voter = env::predecessor_account_id(); 
        post.remove_downvote(voter);
        
        self.posts.insert(&post_id, &post);
    }

    pub fn get_votes_statistics(&self, post_id: usize) -> (usize, usize) {
        let post =  match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };         
        let upvotes = post.get_upvotes().len();
        let downvotes = post.get_downvotes().len();

        (upvotes, downvotes)
    }

    pub fn get_user_vote_status(&self, post_id: usize, user_id: AccountId) -> VoteStatus {
        let post =  match self.posts.get(&post_id) {
            Some(post) => post,
            None => panic!("Post does not exist"),
        };        
        let voter = user_id;
        let upvotes = post.get_upvotes().contains(&voter);
        let downvotes = post.get_downvotes().contains(&voter);

        if upvotes && !downvotes {
            VoteStatus::Upvoted
        } else if !upvotes && downvotes {
            VoteStatus::Downvoted
        } else {
            VoteStatus::None
        }
    }

}

/*
 * The rest of this file holds the inline tests for the code above
 * Learn more about Rust tests: https://doc.rust-lang.org/book/ch11-01-writing-tests.html
 *
 * To run from contract directory:
 * cargo test -- --nocapture
 *
 * From project root, to run in combination with frontend tests:
 * yarn test
 *
 */
#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{MockedBlockchain};
    use near_sdk::{testing_env, VMContext};

    // mock the context for testing, notice "signer_account_id" that was accessed above from env::
    fn get_context(input: Vec<u8>, is_view: bool) -> VMContext {
        VMContext {
            current_account_id: "decentrablog.npmrunstart.testnet".to_string(),
            signer_account_id: "alice_near".to_string(),
            signer_account_pk: vec![0, 1, 2],
            predecessor_account_id: "alice_near".to_string(),
            input,
            block_index: 0,
            block_timestamp: 0,
            account_balance: 1000000000000000000000000,
            account_locked_balance: 0,
            storage_usage: 0,
            attached_deposit: 0,
            prepaid_gas: 10u64.pow(18),
            random_seed: vec![0, 1, 2],
            is_view,
            output_data_receivers: vec![],
            epoch_height: 19,
        }
    }

    #[test]
    fn create_post() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());

        //log id
        env::log(format!("Debug here {}", contract.get_post(0).unwrap().get_post_id()).as_bytes());
        
        assert_eq!(
            "This is the title".to_string(),
            contract.get_post(0).unwrap().get_title()
        );
        assert_eq!(
            "Lets go Brandon!".to_string(),
            contract.get_post(0).unwrap().get_body()
        );
        assert_eq!(1, contract.get_total_posts());
        assert_eq!(0, contract.get_post(0).unwrap().get_post_id());

        //test get_user_posts
        let user_posts = contract.get_user_posts("alice_near".to_string());
        assert_eq!(1, user_posts.len());
        assert_eq!(0, user_posts[0].get_post_id());
    }

    #[test]
    fn delete_a_post_then_add_two_posts() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());
        contract.delete_post(0);
        
        assert_eq!(0, contract.get_total_posts(), "Total posts should be 0");

        // add a post
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());
        assert_eq!(2, contract.get_total_posts());

        //next post id
        assert_eq!(3, contract.get_next_post_id());
    }

    #[test]
    fn return_owner_account_id() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let contract = Blog::default();
        assert_eq!(
            "alice_near".to_string(),
            contract.get_owner()
        );
    }

    #[test]
    fn create_comment() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();

        // Create the first post
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());
        contract.create_comment(0, "This is the comment".to_string());

        assert_eq!(
            "This is the comment".to_string(),
            contract.get_comment(0).get_body()
        );
        assert_eq!(0, contract.get_comment(0).get_comment_id());

        contract.create_comment(0, "This is comment 2, id 1".to_string());
        contract.create_comment(0, "This is comment 3, id 2".to_string());

        // Check if the comments is there
        assert_eq!(
            "This is comment 2, id 1".to_string(),
            contract.get_comment(1).get_body()
        );
        assert_eq!(
            "This is comment 3, id 2".to_string(),
            contract.get_comment(2).get_body()
        );

        let comments = contract.get_post(0).unwrap().get_comments();

        //assert size of comments
        assert_eq!(3, comments.len(), "Comments size is not 3");
        assert_eq!(3, contract.get_post_total_comments(0), "get_post_total_comments is not 3");

        //Check comment string
        assert_eq!(
            "This is comment 2, id 1".to_string(),
            contract.get_comment(1).get_body()
        );
        assert_eq!(
            "This is comment 3, id 2".to_string(),
            contract.get_comment(2).get_body()
        );
    }

    #[test]
    fn upvote_test() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();

        // Create the first post
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());

        // Upvote the post
        contract.upvote(0);

        // Check if the upvote is there
        assert_eq!(
            1,
            contract.get_post(0).unwrap().get_upvotes().len()
        );

        // upvote 10 times 
        for _ in 0..10 {
            contract.upvote(0);
        }

        // downvote 5 times
        for _ in 0..5 {
            contract.downvote(0);
        }

        // check statistic, 10 times of upvote is 1 in an account
        // 5 times of downvote is the same
        assert_eq!(
            (0, 1),
            contract.get_votes_statistics(0)
        );
        
    }

    #[test]
    fn test_paging_post() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();

        // Loop 100 post and create them
        for i in 0..45 {
            contract.create_post(format!("This is the title {}", i), format!("Lets go Brandon! {}", i));
        }

        assert_eq!(45, contract.get_total_posts(), "Total post is not 45");
        
        // Check paging post call
        // let posts = contract.get_paging_posts(1, 10);
        // assert_eq!(10, posts.len(), "Paging post size is not 10");
        // assert_eq!(0, posts[0].get_post_id(), "Paging post id is not 10");
        // assert_eq!(9, posts[9].get_post_id(), "Paging post id is not 20");

        // Check paging post call
        let posts = contract.get_paging_posts(5, 10);

        assert_eq!(5, posts.len(), "Paging post size is not 5");
        assert_eq!(40, posts[0].get_post_id(), "Paging post id is not 40");
        assert_eq!(44, posts[4].get_post_id(), "Paging post id is not 44");
    }

    #[test]
    fn test_donation() {
        let context = get_context(vec![], false);
        testing_env!(context);
        let mut contract = Blog::default();

        // Create the first post
        contract.create_post("This is the title".to_string(), "Lets go Brandon!".to_string());

        // Donate
        contract.donate(0, 1000000, "Support Trump for the USA".to_string());

        // Check if the donation is there
        assert_eq!(
            1,
            contract.get_post(0).unwrap().get_total_donation()
        );
    }
}