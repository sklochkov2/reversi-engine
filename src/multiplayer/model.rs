use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ResponseError {
    pub code: u32,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlayerResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: Vec<User>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct User {
    pub player_uuid: String,
    pub comment: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NewGameRequest {
    pub player_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameRequest {
    pub player_id: String,
    pub game_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NewGameResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: NewGameResult,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AvailableGame {
    pub game_id: String,
    pub first_player: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameListResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: Vec<AvailableGame>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NewGameResult {
    pub game_id: String,
    pub color: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameStatusResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: GameStatusResult,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameStatusResult {
    pub status: String,
    pub last_move: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameJoinResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: GameJoinResult,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GameJoinResult {
    pub result: bool,
    pub color: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveRequest {
    pub player_id: String,
    pub game_id: String,
    pub r#move: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveResult {
    pub ok: bool,
    pub r#continue: bool,
    pub winner: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveResponse {
    pub status: String,
    pub error: ResponseError,
    pub result: MoveResult,
}
