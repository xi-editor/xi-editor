//
//  SearchControler.swift
//  XiEditor
//
//  Created by Christopher Stern on 11/20/16.
//  Copyright © 2016 Raph Levien. All rights reserved.
//

import Cocoa

class SearchController: NSWindowController{
    @IBOutlet var textField: NSTextField!
    @IBOutlet var regExpCheck: NSButton!
    @IBOutlet var caseCheck: NSButton!
    @IBOutlet var wholeWordCheck: NSButton!
    
    @IBAction func findNext(sender: AnyObject) {
        appDelegate?.searchNext()
    }
    
    weak var appDelegate: AppDelegate?
    
    class SearchData: NSObject {
        dynamic var regExp = 0
        dynamic var matchCase = 0
        dynamic var wholeWord = 0
        
        dynamic var text : NSString? = ""
    }
    dynamic var searchData : SearchData = SearchData()
    
    
    convenience init() {
        self.init(windowNibName: "SearchController")
    }

    private var kvoContext: UInt8 = 1

    override func windowDidLoad() {
        
        searchData.addObserver(self, forKeyPath: "regExp", options: NSKeyValueObservingOptions.New, context: &kvoContext)
        searchData.addObserver(self, forKeyPath: "matchCase", options: NSKeyValueObservingOptions.New, context: &kvoContext)
        searchData.addObserver(self, forKeyPath: "wholeWord", options: NSKeyValueObservingOptions.New, context: &kvoContext)
        searchData.addObserver(self, forKeyPath: "text", options: NSKeyValueObservingOptions.New, context: &kvoContext)
        
        regExpCheck.enabled = false
        caseCheck.state=1
        caseCheck.enabled = false
        wholeWordCheck.enabled = false
    }
    
    func setSearchString(s: String) {
        searchData.text = s
    }
    
    override func observeValueForKeyPath(keyPath: String?, ofObject object: AnyObject?, change: [String : AnyObject]?, context: UnsafeMutablePointer<Void>) {
        if context == &kvoContext {
            let searchText : String
            if let sdt = searchData.text {
                searchText = sdt as String
            }
            else {
                searchText = ""
            }
            
           appDelegate?.updateSearch(searchText,
                            regExp:searchData.regExp != 0,
                            matchCase:searchData.matchCase != 0,
                            wholeWord:searchData.wholeWord != 0 )
        }
    }
 
    deinit {
        searchData.removeObserver(self, forKeyPath: "regExp")
        searchData.removeObserver(self, forKeyPath: "matchCase")
        searchData.removeObserver(self, forKeyPath: "wholeWord")
        searchData.removeObserver(self, forKeyPath: "text")
    }
}
