use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::fs::{metadata, read_dir, File};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::ReadHalf;
use tokio::net::tcp::WriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;


#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let port = &args[1];
    let root_folder = &args[2];
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    println!("Root folder: {}", root_folder);
    println!("Server listening on 0.0.0.0:{}", port);

    loop {
        let (socket, addr) = listener.accept().await.unwrap();
        let root_folder = root_folder.clone().to_string();
        tokio::spawn(async move {
            if let Err(e) = handle(socket, root_folder, addr.ip().to_string()).await {
                eprintln!("Error handling connection: {}", e);
            }
        });
    }
}

fn is_readable(metadata: &std::fs::Metadata) -> bool {
    let permissions = metadata.permissions();
    let mode = permissions.mode();
    mode & 0o444 != 0
}

fn split_query_string(path: &str) -> (&str, &str) {
    if let Some(index) = path.find('?') {
        (&path[..index], &path.get(index + 1..).unwrap_or(""))
    } else {
        (&path, "")
    }
}

async fn handle(
    mut socket: TcpStream,
    root_folder: String,
    client_ip: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let (reader, mut writer) = socket.split();
    let mut reader = BufReader::new(reader);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;

    let request_line = request_line.trim();

    let request_parts: Vec<&str> = request_line.split_whitespace().collect();
    if request_parts.len() < 2 {
        write_response(&mut writer, 400, &[], &[]).await?;
        println!("{} {} -> 400 Bad Request", client_ip, request_line);
        return Ok(());
    }

    let method = request_parts[0];
    let (path, params) = split_query_string(request_parts[1]);

    let file_path = format!("{}{}", root_folder, path);

    let root_path_result: Result<PathBuf, _> = fs::canonicalize(&root_folder);
    match root_path_result {
        Ok(root_path) => {
            let requested_file_path = PathBuf::from(&root_folder).join(&path[1..]);
            let canonical_file_path_result = fs::canonicalize(requested_file_path);

            match canonical_file_path_result {
                Ok(canonical_file_path) => {
                    if !canonical_file_path.starts_with(&root_path) {
                        write_response(
                            &mut writer,
                            403,
                            "<html>403 Forbidden</html>".as_bytes(),
                            &[],
                        )
                        .await?;
                        println!("{method} {} {} -> 403 (Forbidden)", client_ip, path);
                        return Ok(());
                    }
                }
                Err(_) => {
                    write_response(
                        &mut writer,
                        404,
                        "<html>404 Not Found</html>".as_bytes(),
                        &[],
                    )
                    .await?;
                    println!("{method} {} {} -> 404 (Not Found)", client_ip, path);
                    return Ok(());
                }
            }
        }
        Err(_) => {
            write_response(
                &mut writer,
                500,
                "<html>500 Internal Server Error</html>".as_bytes(),
                &[],
            )
            .await?;
            println!(
                "{method} {} {} -> 500 (Internal Server Error)",
                client_ip, path
            );
            return Ok(());
        }
    }

    match method {
        "GET" => {
            let metadata = metadata(&file_path).await;
            match metadata {
                Ok(metadata) if is_readable(&metadata) => {
                    if metadata.is_file() {
                        let path_buf = PathBuf::from(&file_path);
                        let extension = path_buf.extension().and_then(std::ffi::OsStr::to_str);

                        match extension {
                            Some("sh") => {
                                let (output, status_code) = run_script(
                                    &root_folder,
                                    path,
                                    params,
                                    &mut reader,
                                    &file_path,
                                    method,
                                )
                                .await?;

                                let lines = output
                                    .split(|b| b == &b'\n')
                                    .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                                    .collect::<Vec<_>>();
                                let lines: Vec<_> = lines.splitn(2, |x| x == &&[]).collect();
                                let (headers, body) = (
                                    lines[0].join("\r\n".as_bytes()),
                                    lines.get(1).unwrap_or(&&[][..]).join("\n".as_bytes()),
                                );

                                write_response(
                                    &mut writer,
                                    status_code,
                                    &body,
                                    &String::from_utf8_lossy(&headers)
                                        .lines()
                                        .collect::<Vec<&str>>(),
                                )
                                .await?;
                                let status = get_status(status_code);
                                println!("GET {} {} -> {status_code} ({status})", client_ip, path);
                            }
                            _ => {
                                let mut file = File::open(&file_path).await?;
                                let mut contents = vec![];
                                file.read_to_end(&mut contents).await?;

                                let content_type = get_content_type(&file_path);
                                let content_length = contents.len();
                                write_response(
                                    &mut writer,
                                    200,
                                    &contents,
                                    &[
                                        &format!("Content-Type: {content_type}"),
                                        &format!("Content-Length: {content_length}"),
                                    ],
                                )
                                .await?;
                                println!("GET {} {} -> 200 (OK)", client_ip, path);
                            }
                        }
                    } else {
                        let mut entries = read_dir(file_path).await?;
                        let mut contents = String::new();
                        while let Some(entry) = entries.next_entry().await? {
                            contents.push_str(&entry.file_name().to_string_lossy());
                            contents.push('\n');
                        }
                        write_response(
                            &mut writer,
                            200,
                            &[],
                            &["Content-type: text/plain; charset=utf-8"],
                        )
                        .await?;
                        println!("GET {} {} -> 200 (OK)", client_ip, path);
                    }
                }
                Ok(_metadata) => {
                    write_response(
                        &mut writer,
                        403,
                        "<html>403 Forbidden</html>".as_bytes(),
                        &[],
                    )
                    .await?;
                    println!("GET {} {} -> 403 (Forbidden)", client_ip, path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    write_response(
                        &mut writer,
                        403,
                        "<html>403 Forbidden</html>".as_bytes(),
                        &[],
                    )
                    .await?;
                    println!("GET {} {} -> 403 (Forbidden)", client_ip, path);
                }
                Err(_) => {
                    write_response(&mut writer, 404, &[], &[]).await?;
                    println!("GET {} {} -> 404 (Not Found)", client_ip, path);
                }
            }
        }
        "POST" => {
            if !path.starts_with("/scripts/") {
                write_response(&mut writer, 404, &[], &[]).await?;
                println!("POST {} {} -> 404 (Not Found)", client_ip, path);
                return Ok(());
            }
            let (output, status_code) =
                run_script(&root_folder, path, params, &mut reader, &file_path, method).await?;

            let lines = output
                .split(|b| b == &b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                .collect::<Vec<_>>();
            let lines: Vec<_> = lines.splitn(2, |x| x == &&[]).collect();
            let (headers, body) = (
                lines[0].join("\r\n".as_bytes()),
                lines.get(1).unwrap_or(&&[][..]).join("\n".as_bytes()),
            );

            write_response(
                &mut writer,
                status_code,
                &body,
                &String::from_utf8_lossy(&headers)
                    .lines()
                    .collect::<Vec<&str>>(),
            )
            .await?;
            let status = get_status(status_code);
            println!("POST {} {} -> {status_code} ({status})", client_ip, path);
        }
        _ => {
            write_response(&mut writer, 405, &[], &[]).await?;
            println!("POST {} {} -> 405 (Method Not Allowed)", client_ip, path);
        }
    }

    Ok(())
}

fn get_content_type(path: &str) -> &str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".zip") {
        "application/zip"
    } else if path.ends_with(".txt") || path.ends_with(".sh") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

fn get_status(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => {
            eprintln!("Invalid status code");
            ""
        }
    }
}

// async fn write_response(
//     writer: &mut WriteHalf<'_>,
//     status_code: u16,
//     body: &[u8],
//     additional_headers: &[&str],
// ) -> Result<(), Box<dyn std::error::Error>> {
//     let status = get_status(status_code);
//     let mut headers = String::new();
//     for header in additional_headers {
//         headers.push_str("\r\n");
//         headers.push_str(header)
//     }
//     let status_line =
//         format!("HTTP/1.0 {status_code} {status}{headers}\r\nConnection: close\r\n\r\n");
//     writer.write_all(status_line.as_bytes()).await?;
//     writer.write_all(&body).await?;
//     Ok(())
// }


async fn write_response(
    writer: &mut WriteHalf<'_>,
    status_code: u16,
    body: &[u8],
    additional_headers: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let status = get_status(status_code);
    let mut headers = String::new();
    if status_code != 500 {
        for header in additional_headers {
            headers.push_str("\r\n");
            headers.push_str(header)
        }
    }
    let status_line =
        format!("HTTP/1.0 {status_code} {status}{headers}\r\nConnection: close\r\n\r\n");
    writer.write_all(status_line.as_bytes()).await?;
    writer.write_all(&body).await?;
    Ok(())
}


async fn run_script<'a>(
    root_folder: &str,
    path: &str,
    params: &str,
    reader: &mut BufReader<ReadHalf<'_>>,
    file_path: &str,
    method: &str,
) -> Result<(Vec<u8>, u16), Box<dyn std::error::Error>> {
    let script_path = PathBuf::from(&root_folder).join(&path[1..]);

    if !script_path.exists() || !is_readable(&fs::metadata(&script_path)?) {
        return Ok((Vec::new(), 404));
    }

    let query_params = params.split('&').filter(|x| x != &"");
    let mut headers = std::collections::HashMap::new();
    loop {
        let mut header_line = String::new();
        reader.read_line(&mut header_line).await?;
        let header_line = header_line.trim();
        if header_line == "" {
            break;
        }
        let mut header_line = header_line.split(':');
        if let (Some(key), Some(value)) = (header_line.next(), header_line.next()) {
            headers.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    let body = if method == "POST" {
        let content_length = headers
            .get("Content-Length")
            .or_else(|| headers.get("Content-length"))
            .and_then(|x| x.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        if content_length != 0 {
            reader.read(&mut body).await?;
        }
        body
    } else {
        Vec::new()
    };
    let output = Command::new("bash")
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .arg(&file_path)
        .envs(
            query_params
                .filter_map(|v| {
                    let mut kv = v.split('=');
                    Some((kv.next()?, kv.next()?))
                })
                .map(|x| ("Query_".to_owned() + x.0, x.1)),
        )
        .envs(headers)
        .env("Method", method)
        .env("Path", path)
        .spawn();
    let output = match output {
        Ok(mut child) => {
            if method == "POST" {
                if let Some(stdin) = &mut child.stdin {
                    stdin.write_all(&body).await.ok();
                };
            }
            child.wait_with_output().await
        }
        Err(e) => Err(e),
    };
    Ok(match output {
        Ok(output) => {
            if output.status.success() {
                (output.stdout, 200)
            } else {
                (output.stderr, 500)
            }
        }
        Err(_) => (Vec::new(), 404),
    })
}
