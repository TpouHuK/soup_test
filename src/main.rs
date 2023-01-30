use anyhow::Result;
use anyhow::{anyhow, Context};
use rand::prelude::*;
use reqwest::StatusCode;
use reqwest::{self, Client};
//use rust_decimal::prelude::*;
use num_traits::cast::ToPrimitive;
use scraper::{Html, Selector};
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::types::Decimal;
//use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, Sqlite};
use once_cell::sync::OnceCell;
use sqlx::ConnectOptions;
use std::str::FromStr;
use tokio::sync::Mutex;

const BASE_URL: &str = "https://www.fashionnova.com";
const URL_MEN_MENU: &str = "https://www.fashionnova.com/pages/men";

//static DATABASE: OnceCell<Mutex<SqliteConnection>> = OnceCell::new();
static DATABASE: OnceCell<Mutex<PgConnection>> = OnceCell::new();

async fn get_bs(url: &str) -> Result<Html> {
    let client = Client::new();
    let response = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/109.0",
        )
        .send()
        .await?;
    println!("{url}");
    if response.status() != StatusCode::OK {
        return Err(anyhow!("Bad status: {:?}", response.status()));
    }
    let content = response.text().await?;
    //println!(content);
    Ok(Html::parse_document(&content))
}

async fn get_links(url: &str, left: usize, right: usize) -> Vec<(String, String)> {
    let bs = get_bs(url).await.unwrap();

    // find_all("a", class_="menu-category__link")[left:right];
    let selector = &Selector::parse("a.menu-category__link").unwrap();
    let link_elements = bs.select(selector).skip(left).take(right - left);
    // [el.attrs["href"] for el in link_elements];
    let links: Vec<String> = link_elements
        .map(|el| el.value().attr("href").unwrap().to_string())
        .collect();

    // bs.find_all("div", class_="menu-category__item-title")[left:right];
    let selector = &Selector::parse("div.menu-category__item-title").unwrap();
    let category_elements = bs.select(selector).skip(left).take(right - left);

    // [str(el.string).strip() for el in category_elements];
    let categories: Vec<String> = category_elements
        .map(|el| el.text().collect::<String>().trim().to_string())
        .collect();

    links.into_iter().zip(categories.into_iter()).collect()
}

fn parse_price_1(bs: &Html) -> Result<Decimal> {
    // find("div", class_="product-info__price-line");
    let selector = &Selector::parse("div.product-info__price-line").unwrap();
    let price_element = bs.select(selector).next().unwrap();
    for child in price_element.children() {
        println!("child: {:?}", child.value());
    }

    let price_string = price_element.html();

    println!("price_string {price_string:?}");
    println!("price_innerhtml {:?}", price_element.inner_html());

    let price_string_collect = price_element.text().collect::<String>().trim().to_string();
    println!("price_string_collect: {:?}", price_string_collect);
    let mut price_li = price_string.find('$').context("Bad price")?;

    //let price_ri1 = price_string.find("&", price_li);
    let price_ri1 = price_string[price_li..].find('&');
    let price_ri2 = price_string[price_li..].find('<');

    let price_ri = price_li
        + match (price_ri1, price_ri2) {
            (None, None) => {
                return Err(anyhow!("bad price"));
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => a.min(b),
        };

    price_li += 1;
    let price = &price_string[price_li..price_ri];

    let price = match Decimal::from_str_exact(price) {
        Ok(price) => price,
        Err(_) => {
            println!("invalid_price");
            println!("{price}");
            return Err(anyhow!("bad_price"));
        }
    };
    Ok(price)
}

async fn parse_product(link: &str, category: &str) -> Result<()> {
    let url = BASE_URL.to_owned() + link;
    let (name, price, description, img, left) = {
        let bs = { get_bs(&url).await? };

        // find("h1", class_="product-info__title")
        let selector = &Selector::parse("h1.product-info__title").unwrap();
        let name = {
            let name_element = bs.select(selector).next().context("No name")?;
            name_element.text().collect::<String>().trim().to_string()
        };

        // .select_one("div.product-info__details-body > ul")
        let price = {
            let mut price = parse_price_1(&bs);
            if price.is_err() {
                let selector = &Selector::parse("span.price").unwrap();
                let price_element = bs.select(selector).next().unwrap();
                let price_text = price_element.text().collect::<String>().trim().to_string();
                price = Ok(Decimal::from_str(&price_text)?);
            }
            price.context("Bad price")?
        };

        let description = {
            let selector = &Selector::parse("div.product-info__details-body > ul").unwrap();
            let description_element = bs.select(selector).next().context("No descr")?;
            description_element
                .text()
                .collect::<String>()
                .trim()
                .to_string()
        };

        //find("button", class_="product-slideshow__syte-button syte-discovery-modal")
        let img = {
            let selector =
                &Selector::parse("button.product-slideshow__syte-button.syte-discovery-modal")
                    .unwrap();
            let image_element = bs.select(selector).next().unwrap();
            image_element
                .value()
                .attr("data-image-src")
                .unwrap()
                .to_owned()
        };

        let mut rand = thread_rng();
        let left = rand.gen_range(0..20);

        (name, price, description, img, left)
    };

    println!("name:{name}\ndesc:{description}\ncategory:{category}\nprice:{price:?}\nleft:{left}\nimg:{img}");

    //cur.execute("""INSERT INTO api_product (name, description, category, price, "left", img)
    //VALUES (%s, %s, %s, %s, %s, %s);""",
    //(name, description, category, price, left, img))

    // VALUES ($0, $1, $2, $3, $4, $5)
    // VALUES (?, ?, ?, ?, ?, ?)
    //let price: f32 = price.to_f32().unwrap();

    let mut transaction = DATABASE.get().unwrap().lock().await;

    sqlx::query!(
        "
            INSERT INTO api_product (name, description, category, price, \"left\", img)
            VALUES ($1, $2, $3, $4, $5, $6)
        ",
        name,
        description,
        category,
        price,
        left,
        img
    )
    .execute(&mut (*transaction))
    .await
    .unwrap();
    Ok(())
}

async fn parse_products<'a>(link: &str, category: &str) {
    println!("Parsing {category}");
    let mut rand = thread_rng();

    let url = BASE_URL.to_owned() + link;
    let bs = get_bs(&url).await.unwrap();
    let number_of_products = rand.gen_range(12..24);

    // select("div.product-tile__product-title > a")[:number_of_products]
    let selector = &Selector::parse("div.product-tile__product-title > a").unwrap();
    let link_elements = bs.select(selector).take(number_of_products);
    // [el.attrs["href"] for el in link_elements]
    let links: Vec<String> = link_elements
        .map(|el| el.value().attr("href").unwrap().to_string())
        .collect();

    let mut set = tokio::task::JoinSet::new();
    for link in links {
        let category_clone = category.to_owned();
        //parse_product(&link, &category_clone).await;
        set.spawn(async move {
            parse_product(&link, &category_clone).await;
        });
        //println!("{result:?}");
    }
    while let Some(res) = set.join_next().await {}
}

#[tokio::main]
async fn main() {
    /*let conn = SqliteConnectOptions::from_str("sqlite:/home/tpouhuk/Work/soup_test/database.sqlite")
    .unwrap()
    .connect().await.unwrap();*/

    let conn = PgConnectOptions::new()
        .host("localhost")
        .port(5432)
        .database("webstoredb")
        .username("postgres")
        .password("admin")
        .connect()
        .await
        .unwrap();

    let db = Mutex::new(conn);
    DATABASE.set(db);

    let women_links = get_links(BASE_URL, 5, 17).await;
    let men_links = get_links(URL_MEN_MENU, 4, 11).await;

    println!("Parsing Women's categories");
    for (link, category) in women_links {
        parse_products(&link, &format!("Women {category}")).await;
    }

    println!("Parsing Men's categories");
    for (link, category) in men_links {
        println!("{link}, {category}");
        parse_products(&link, &format!("Men {category}")).await;
    }

    //conn.commit();
    //cur.close();
    //conn.close();
}
