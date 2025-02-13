use boa_engine::{Context, Source, property::Attribute, js_string};
use boa_runtime::Console;

struct MyJsCode; // 空構造体を定義

trait JsCode { // JSのコードをtrait impl にしてみた
  fn date(&self) -> String;
  fn hello(&mut self) -> String;
}

impl JsCode for MyJsCode { // dyn は dynamic 
  fn date(&self) -> String {
    "new Date().toString();".to_string()
  }

  fn hello(&mut self) -> String {
    "console.log('Hello')".to_string()
  }
}

fn js_code<T: JsCode>(f: &mut T) {

    let mut context = Context::default();

    let console = Console::init(&mut context);

    context
      .register_global_property(js_string!(Console::NAME), console, Attribute::all())
      .expect("the console object shouldn't exist yet");

      let script = format!("{};{}", f.date(), f.hello());
    
    let result = context.eval(Source::from_bytes(script.as_str())); 
    // Context の eval method で JS コード評価
    
    match result {
      Ok(value) => {
          let date_str = value.to_string(&mut context).unwrap().to_std_string_escaped();
          println!("{}", date_str);
      }
      Err(e) => {
          eprintln!("Error: {}", e);
      }
  }
}

fn main() {
  let mut js = MyJsCode;
  js_code(&mut js);
}