// Immutable handcrafted review flow, embedded at compile time.
pub fn prepare(request: Request) -> Result<Review, Error> {
    let token = request.token();
    let session = auth.validate(token)?;
    let key = session.cache_key();
    let snapshot = store.load(key)?;
    if snapshot.is_current() {
        return Ok(snapshot.review());
    } else {
        return Err(Error::Stale);
    }
}
pub fn validate(token: Token) -> Result<Session, Error> {
    token.check_expiry()?;
    sessions.lookup(token.subject())
}
pub fn audit(review: &Review) {
    log::info!("review prepared: {}", review.id());
}
