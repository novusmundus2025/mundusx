//! Progressive disclosure of extension skill metadata, never skill contents or
//! host/workspace instructions. Keep a stable, complete name/path index.
use serde_json::{json,Value};
use std::collections::BTreeSet;

fn field<'a>(s:&mut &'a str,name:&str)->Option<&'a str> {
    let rest=s.trim_start().strip_prefix(&format!("<{name}>"))?;
    let (value,tail)=rest.split_once(&format!("</{name}>"))?;*s=tail;Some(value)
}
fn words(s:&str)->BTreeSet<String> {
    s.split(|c:char|!c.is_alphanumeric()).filter(|w|w.chars().count()>=4)
        .map(str::to_ascii_lowercase)
        .filter(|w|!["this","that","with","from","when","using","must","should","have","will","files","file","code","project","test","tests","create","read","tool","tools","user","your","into","only","same","then","such","does","their","which","each","data","existing","before","after","return","preserve","complete"].contains(&w.as_str())).collect()
}
pub fn compact(original:&Value,out:&mut Value) {
    let Some(task)=super::native_task_progress::task(original) else {return;};
    let task_words=words(task);let lower_task=task.to_ascii_lowercase();let mut extra=None;
    for message in out["messages"].as_array_mut().into_iter().flatten() {
        if message["role"]!="system" {continue;}
        let Some(text)=message["content"].as_str().filter(|s|s.starts_with("You are an expert AI programming assistant, working with a user in the VS Code editor.")) else {continue;};
        let Some((before,tail))=text.split_once("<skills>") else {continue;};
        let Some((catalog,after))=tail.split_once("</skills>") else {continue;};
        let mut rest=catalog;let mut entries=Vec::new();let mut valid=true;
        while let Some((prefix,tail))=rest.split_once("<skill>") {
            let Some((body,end))=tail.split_once("</skill>") else {valid=false;break;};
            let mut body_rest=body;
            let Some((name,description,path))=(||Some((field(&mut body_rest,"name")?,field(&mut body_rest,"description")?,field(&mut body_rest,"file")?)))() else {valid=false;break;};
            if !body_rest.trim().is_empty()||name.contains(['<','>','\n','\r'])||path.contains(['<','>','\n','\r']) {valid=false;break;}
            let extension=path.replace('\\',"/").to_ascii_lowercase().contains("/.vscode/extensions/");
            let score=words(&format!("{name} {description}")).intersection(&task_words).count();
            let explicit=lower_task.contains(&name.to_ascii_lowercase());
            entries.push((prefix,body,name,description,path,extension,score,explicit));rest=end;
        }
        if !valid||entries.iter().filter(|e|e.5).count()<8 {continue;}
        let mut ranked=entries.iter().enumerate().filter(|(_,e)|e.5&&e.6>=2).map(|(i,e)|(i,e.6)).collect::<Vec<_>>();
        ranked.sort_by_key(|(i,score)|(std::cmp::Reverse(*score),*i));
        let selected=ranked.into_iter().take(3).map(|e|e.0).collect::<BTreeSet<_>>();
        let mut index=String::new();let mut relevant=Vec::new();
        for (i,(prefix,body,name,description,path,extension,_,explicit)) in entries.iter().enumerate() {
            index.push_str(prefix);
            if !extension {index.push_str("<skill>");index.push_str(body);index.push_str("</skill>");continue;}
            index.push_str(&format!("<skill><name>{name}</name><file>{path}</file></skill>"));
            if *explicit||selected.contains(&i) {relevant.push(json!({"name":name,"description":description,"file":path}));}
        }
        index.push_str(rest);
        let note="\nExtension skill descriptions are deferred for context efficiency. Every skill name and exact file path remains indexed. Read a skill file for its full domain, applicability and instructions before using it; deferred metadata does not disable that skill. Built-in and user-authored skill metadata, host instructions and workspace rules remain intact.\n";
        message["content"]=json!(format!("{before}<skills>{note}{index}</skills>{after}"));
        if !relevant.is_empty() {
            // Append after the stable first system/tool prefix, preserving KV
            // cache reuse when the task selects different skill descriptions.
            extra=Some(json!({"role":"system","name":"mundusx_relevant_skill_metadata","content":format!("Extension skill discovery metadata, not executable instructions. These descriptions help select a skill; read its file for the complete instructions before use. All original host permissions and workspace rules remain in force: {}",json!(relevant))}));
        }
        break;
    }
    if let Some(extra)=extra {out["messages"].as_array_mut().unwrap().push(extra);}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(task:&str)->Value {
        let skills=(0..10).map(|i|format!("<skill><name>python-task-{i}</name><description>Python typing annotations and detailed discovery metadata {i}</description><file>C:/Users/u/.vscode/extensions/pylance/skills/{i}/SKILL.md</file></skill>")).collect::<String>();
        json!({"messages":[{"role":"system","content":format!("You are an expert AI programming assistant, working with a user in the VS Code editor.\nKEEP_HOST_PERMISSION\n<skills>KEEP_DISCOVERY_INSTRUCTIONS{skills}</skills>\nKEEP_WORKSPACE_RULE")},{"role":"user","content":format!("<userRequest>{task}</userRequest>")}]})
    }
    #[test] fn metadata_deferred_but_every_path_and_real_instruction_remains() {
        let p=p("Edit a Node.js HTTP handler");let original=p.clone();let mut out=p.clone();compact(&p,&mut out);assert_eq!(p,original);
        let text=out["messages"][0]["content"].as_str().unwrap();assert!(!text.contains("detailed discovery metadata"));
        for value in ["KEEP_HOST_PERMISSION","KEEP_DISCOVERY_INSTRUCTIONS","KEEP_WORKSPACE_RULE"] {assert!(text.contains(value));}
        for i in 0..10 {assert!(text.contains(&format!("python-task-{i}")));assert!(text.contains(&format!("pylance/skills/{i}/SKILL.md")));}
    }
    #[test] fn explicit_skills_remain_discoverable_and_task_selection_keeps_prefix_stable() {
        let a=p("Use python-task-8");let b=p("Use python-task-9");let mut av=a.clone();let mut bv=b.clone();compact(&a,&mut av);compact(&b,&mut bv);
        assert_eq!(av["messages"][0],bv["messages"][0]);
        assert!(av["messages"].as_array().unwrap().iter().any(|m|m["name"]=="mundusx_relevant_skill_metadata"&&m["content"].as_str().unwrap().contains("metadata 8")));
    }
    #[test] fn malformed_or_unrecognized_catalogs_are_untouched() {
        let mut p=p("Edit source");p["messages"][0]["content"]=json!(p["messages"][0]["content"].as_str().unwrap().replace("</file>","</wrong>"));let mut out=p.clone();compact(&p,&mut out);assert_eq!(p,out);
        let mut p=p.clone();p["messages"][0]["content"]=json!("An unrelated system prompt <skills>must remain</skills>");let mut out=p.clone();compact(&p,&mut out);assert_eq!(p,out);
    }
    #[test] fn tool_free_greetings_keep_preferences_without_the_full_extension_catalog() {
        let mut p=p("hello");p["tools"]=json!([]);p["tool_choice"]=json!("none");
        p["messages"].as_array_mut().unwrap().push(json!({"role":"system","name":"mundusx_conversational_turn","content":"Answer the greeting; retain the user's preferences"}));
        let original=p.clone();let out=super::super::native_line_edit::prepare(&p);assert_eq!(p,original);
        assert!(out["messages"][0]["content"].as_str().unwrap().len()<p["messages"][0]["content"].as_str().unwrap().len());
        assert!(out.to_string().contains("retain the user's preferences"));assert_eq!(out["tool_choice"],"none");assert!(out["tools"].as_array().unwrap().is_empty());
    }
}
