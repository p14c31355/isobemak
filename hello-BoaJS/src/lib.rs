use boa_engine::{Context, Source, };


pub fn log() {
  
  let mut context2 = Context::default();

  

  
  
  let result2 = context2.eval(Source::from_bytes(js_code2));

  match result2 {
    Ok(res) => println!("{}", res.to_string(&mut context2).unwrap().to_std_string_escaped()),
    Err(e) => eprintln!("Uncaught {e}")
  };
}