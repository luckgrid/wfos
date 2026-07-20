use starbase::MainResult;
use takogami::app;

#[tokio::main]
async fn main() -> MainResult {
    app::run().await
}
