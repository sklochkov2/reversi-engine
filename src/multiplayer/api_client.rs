use std::{thread, time};

use crate::multiplayer::model::*;

use crate::cli::args::*;

pub fn find_games_to_join(args: &Args) -> Result<Vec<String>, ureq::Error> {
    let mut res: Vec<String> = Vec::new();
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/game_list";
    println!("{}", api_endpoint);
    let join_request: NewGameRequest = NewGameRequest {
        player_id: args.player_uuid.clone(),
    };
    let list_games_result: GameListResponse = ureq::post(api_endpoint.as_str())
        .send_json(&join_request)?
        .body_mut()
        .read_json::<GameListResponse>()?;
    for game in list_games_result.result {
        if game.first_player != args.player_uuid {
            res.push(game.game_id);
        }
    }
    Ok(res)
}

pub fn create_game(args: &Args) -> Result<NewGameResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/create_game";
    let create_request: NewGameRequest = NewGameRequest {
        player_id: args.player_uuid.clone(),
    };
    let created_game: NewGameResponse = ureq::post(api_endpoint.as_str())
        .send_json(&create_request)?
        .body_mut()
        .read_json::<NewGameResponse>()?;
    Ok(created_game.result)
}

pub fn join_game(args: &Args, game_uuid: String) -> Result<GameJoinResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/join";
    let game_request: GameRequest = GameRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
    };
    let joined_game: GameJoinResponse = ureq::post(api_endpoint.as_str())
        .send_json(&game_request)?
        .body_mut()
        .read_json::<GameJoinResponse>()?;
    Ok(joined_game.result)
}

pub fn make_move(args: &Args, game_uuid: String, our_move: String) -> Result<MoveResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/move";
    let move_request: MoveRequest = MoveRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
        r#move: our_move.clone(),
    };
    let move_response: MoveResponse = ureq::post(api_endpoint.as_str())
        .send_json(&move_request)?
        .body_mut()
        .read_json::<MoveResponse>()?;
    Ok(move_response.result)
}

pub fn get_game_status(args: &Args, game_uuid: String) -> Result<GameStatusResult, ureq::Error> {
    let api_endpoint: String = args.api_url.clone() + "reversi/v1/game_status";
    let game_request: GameRequest = GameRequest {
        player_id: args.player_uuid.clone(),
        game_id: game_uuid.clone(),
    };
    let status: GameStatusResponse = ureq::post(api_endpoint.as_str())
        .send_json(&game_request)?
        .body_mut()
        .read_json::<GameStatusResponse>()?;
    Ok(status.result)
}

pub fn wait_for_response(args: &Args, game_uuid: String, my_color: String) -> GameStatusResult {
    loop {
        let curr_result: GameStatusResult;
        match get_game_status(args, game_uuid.clone()) {
            Ok(g) => {
                curr_result = g;
            }
            Err(e) => {
                println!("Failed to fetch game status, retrying: {}", e);
                thread::sleep(time::Duration::from_millis(1000));
                continue;
            }
        }
        if curr_result.status == my_color
            || curr_result.status == "black_won".to_string()
            || curr_result.status == "white_won".to_string()
        {
            return curr_result;
        }
        thread::sleep(time::Duration::from_millis(500));
    }
}

pub fn wait_for_joining_player(args: &Args, game_uuid: String) -> GameStatusResult {
    loop {
        let curr_result: GameStatusResult;
        match get_game_status(args, game_uuid.clone()) {
            Ok(g) => {
                curr_result = g;
            }
            Err(e) => {
                println!("Failed to fetch game status, retrying: {}", e);
                thread::sleep(time::Duration::from_millis(1000));
                continue;
            }
        }
        if curr_result.status != "pending".to_string() {
            return curr_result;
        }
        thread::sleep(time::Duration::from_millis(500));
    }
}
